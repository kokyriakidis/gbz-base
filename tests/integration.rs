//! Integration tests for binaries.

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output, ExitStatus};

use gbz_base::GBZBase;
use gbz_base::{GAFBase, GAFBaseParams, GraphReference};
use gbz_base::gaf_sort::{sort_gaf, SortParameters, KeyType};
use gbz_base::utils;

use gbz::GBZ;
use simple_sds::serialize;

//-----------------------------------------------------------------------------

// TODO: Should this be in a library somewhere?
struct TempFileHandler {
    files: Vec<PathBuf>,
}

impl TempFileHandler {
    fn new() -> Self {
        TempFileHandler { files: Vec::new() }
    }

    fn add_file(&mut self, file: &PathBuf) {
        self.files.push(file.clone());
    }

    fn new_file(&mut self, name_part: &str) -> PathBuf {
        let file = serialize::temp_file_name(name_part);
        assert!(!file.exists(), "Temporary file {} already exists", file.display());
        self.files.push(file.clone());
        file
    }

    fn create_copy(&mut self, source: &PathBuf) -> PathBuf {
        let copy = serialize::temp_file_name("copy");
        assert!(!copy.exists(), "Temporary file {} already exists", copy.display());
        fs::copy(source, &copy).expect("Failed to copy file");
        self.files.push(copy.clone());
        copy
    }
}

impl Drop for TempFileHandler {
    fn drop(&mut self) {
        for file in &self.files {
            let _ = fs::remove_file(file);
        }
    }
}

fn compare_files(file1: &PathBuf, file2: &PathBuf) -> bool {
    let content1 = fs::read(file1).expect(&format!("Failed to read file 1: {}", file1.display()));
    let content2 = fs::read(file2).expect(&format!("Failed to read file 2: {}", file2.display()));
    content1 == content2
}

fn count_lines_in_file(file: &PathBuf) -> usize {
    let content = fs::read_to_string(file).expect(&format!("Failed to read file: {}", file.display()));
    content.lines().count()
}

fn count_lines(data: &[u8]) -> usize {
    let content = String::from_utf8_lossy(data);
    content.lines().count()
}

//-----------------------------------------------------------------------------

fn get_binary_path(binary_name: &str) -> std::path::PathBuf {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("target");
    path.push("debug");
    path.push(binary_name);
    path
}

fn build_gbz_base_truth(
    temp_files: &mut TempFileHandler,
    graph_file: &PathBuf, chains_file: Option<&PathBuf>
) -> PathBuf {
    let truth_file = temp_files.new_file("gbz-base-truth");
    let result = GBZBase::create_from_files(
        graph_file, chains_file.map(|p| p.as_path()), &truth_file
    );
    assert!(result.is_ok(), "Failed to create truth database: {}", result.unwrap_err());
    truth_file
}

fn sort_gaf_to_temp(temp_files: &mut TempFileHandler, input_file: &PathBuf) -> PathBuf {
    let sorted_file = temp_files.new_file("sorted");
    sort_gaf(input_file, &sorted_file, &SortParameters::default())
        .expect("Failed to sort GAF file");
    sorted_file
}

fn build_gaf_base_truth(
    temp_files: &mut TempFileHandler,
    sorted_input: &PathBuf, gbwt_file: Option<&PathBuf>, graph_file: Option<&PathBuf>,
    params: &GAFBaseParams
) -> PathBuf {
    let truth_file = temp_files.new_file("gaf-base-truth");

    let result = if let Some(graph_file) = graph_file {
        let graph: GBZ = serialize::load_from(graph_file).expect("Failed to read graph file");
        GAFBase::create_from_files(
            sorted_input, gbwt_file.map(|p| p.as_path()), &truth_file,
            GraphReference::Gbz(&graph), params
        )
    } else {
        GAFBase::create_from_files(
            sorted_input, gbwt_file.map(|p| p.as_path()), &truth_file,
            GraphReference::None, params
        )
    };
    assert!(result.is_ok(), "Failed to create truth database: {}", result.unwrap_err());

    truth_file
}

//-----------------------------------------------------------------------------

// Tests for `gbz-base construct`.

// We assume that the graph file is a temporary file if no output file is provided.
// Otherwise there may be conflicts with multiple tests running in parallel.
fn run_gbz_base_construct(
    temp_files: &mut TempFileHandler,
    graph_file: &PathBuf, chains_file: Option<&PathBuf>,
    overwrite: bool, output_file: Option<&PathBuf>
) -> (PathBuf, ExitStatus) {
    let mut args = vec![String::from("construct")];
    let output_file = match output_file {
        Some(path) => {
            args.push(String::from("--output"));
            args.push(path.to_str().unwrap().to_string());
            path.clone()
        },
        None => {
            let path = graph_file.with_added_extension("db");
            temp_files.add_file(&path);
            path
        },
    };
    if overwrite {
        args.push(String::from("--overwrite"));
    }
    if let Some(chains_file) = chains_file {
        args.push(String::from("--chains"));
        args.push(chains_file.to_str().unwrap().to_string());
    }
    args.push(graph_file.to_str().unwrap().to_string());

    let binary = get_binary_path("gbz-base");
    let result = Command::new(binary)
        .args(&args)
        .output()
        .expect("Failed to execute gbz-base construct");
    (output_file, result.status)
}

#[test]
fn gbz_base_construct() {
    let mut temp_files = TempFileHandler::new();
    let original_graph_file = utils::get_test_data("micb-kir3dl1.gbz");
    let graph_file = temp_files.create_copy(&original_graph_file);
    let chains_file = None;

    // We expect that the GBZ-base built with `gbz-base construct` is identical to
    // one built with a direct library call.
    let truth_file = build_gbz_base_truth(&mut temp_files, &graph_file, chains_file);
    let (output_file, status) = run_gbz_base_construct(
        &mut temp_files, &graph_file, chains_file, false, None
    );
    assert!(status.success(), "gbz-base construct failed with status: {}", status);
    assert!(compare_files(&output_file, &truth_file), "Output file does not match truth file");
    let output_metadata = fs::metadata(&output_file).expect("Failed to read output file metadata");
    let old_timestamp = output_metadata.modified().expect("Failed to get output file timestamp");

    // Try rebuilding the database without overwrite, expect failure.
    let (_, status) = run_gbz_base_construct(
        &mut temp_files, &graph_file, chains_file, false, None
    );
    assert!(!status.success(), "gbz-base construct should have failed without overwrite");

    // Rebuild with overwrite, expect success.
    let (_, status) = run_gbz_base_construct(
        &mut temp_files, &graph_file, chains_file, true, None
    );
    assert!(status.success(), "gbz-base construct failed with overwrite with status: {}", status);

    // Check that we did actually overwrite the output file.
    let output_metadata = fs::metadata(&output_file).expect("Failed to read output file metadata");
    let new_timestamp = output_metadata.modified().expect("Failed to get output file timestamp");
    assert!(new_timestamp > old_timestamp, "Output file timestamp did not change after overwrite");
    assert!(compare_files(&output_file, &truth_file), "Output file does not match truth file after overwrite");

    // Try specifying an explicit output file name.
    let explicit_output_file = temp_files.new_file("gbz-base-output");
    let (_, status) = run_gbz_base_construct(
        &mut temp_files, &graph_file, chains_file, false, Some(&explicit_output_file)
    );
    assert!(status.success(), "gbz-base construct failed with explicit output file with status: {}", status);
    assert!(compare_files(&explicit_output_file, &truth_file), "Explicit output file does not match truth file");
}

#[test]
fn gbz_base_construct_with_chains() {
    let mut temp_files = TempFileHandler::new();
    let original_graph_file = utils::get_test_data("micb-kir3dl1.gbz");
    let graph_file = temp_files.create_copy(&original_graph_file);
    let chains_file = Some(utils::get_test_data("micb-kir3dl1.chains"));

    let truth_file = build_gbz_base_truth(&mut temp_files, &graph_file, chains_file.as_ref());
    let (output_file, status) = run_gbz_base_construct(
        &mut temp_files, &graph_file, chains_file.as_ref(), false, None
    );
    assert!(status.success(), "gbz-base construct failed with status: {}", status);
    assert!(compare_files(&output_file, &truth_file), "Output file does not match truth file");
}

//-----------------------------------------------------------------------------

// Tests for `gaf-base sort`.

fn run_gaf_base_sort(input_file: &PathBuf, output_file: Option<&PathBuf>, params: &SortParameters) -> Output {
    let mut args = vec![String::from("sort")];
    if let Some(output_file) = output_file {
        args.push(String::from("--output"));
        args.push(output_file.to_str().unwrap().to_string());
    }
    args.push(String::from("--key-type"));
    match params.key_type {
        KeyType::NodeInterval => args.push(String::from("interval")),
        KeyType::Hash => args.push(String::from("hash")),
    }
    args.push(String::from("--records-per-file"));
    args.push(params.records_per_file.to_string());
    args.push(String::from("--files-per-merge"));
    args.push(params.files_per_merge.to_string());
    args.push(String::from("--buffer-size"));
    args.push(params.buffer_size.to_string());
    args.push(String::from("--threads"));
    args.push(params.threads.to_string());
    if params.stable {
        args.push(String::from("--stable"));
    }
    if params.progress {
        args.push(String::from("--progress"));
    }
    args.push(input_file.to_str().unwrap().to_string());

    let binary = get_binary_path("gaf-base");
    Command::new(binary)
        .args(&args)
        .output()
        .expect("Failed to execute gaf-base sort")
}

fn count_merge_rounds(stderr: &[u8]) -> usize {
    let stderr_str = String::from_utf8_lossy(stderr);
    let lines = stderr_str.lines();
    let mut rounds = 0;
    for line in lines {
        if line.starts_with("Round") {
            if line.ends_with("files per batch") {
                rounds += 1;
            }
        } else if line.starts_with("Starting the final merge") {
            rounds += 1;
        }
    }
    rounds
}

#[test]
fn gaf_base_sort() {
    let mut temp_files = TempFileHandler::new();
    let input_file = utils::get_test_data("shuffled.gaf");
    let params = SortParameters::default();

    let truth_file = temp_files.new_file("sorted");
    sort_gaf(&input_file, &truth_file, &params).expect("Failed to sort GAF file for truth");
    let truth = fs::read(&truth_file).expect("Failed to read truth file");

    // Check that the default arguments correspond to the default parameters.
    let default_args = vec![String::from("sort"), input_file.to_str().unwrap().to_string()];
    let binary = get_binary_path("gaf-base");
    let default_output = Command::new(&binary)
        .args(&default_args)
        .output()
        .expect("Failed to execute gaf-base sort with default args");
    assert!(default_output.status.success(), "gaf-base sort failed with default args");
    let correct_output = default_output.stdout == truth; // These are large, so we don't want to print them out.
    assert!(correct_output, "gaf-base sort produced incorrect output with default args");
    assert!(default_output.stderr.is_empty(), "gaf-base sort produced stderr output with default args");

    let explicit_output = run_gaf_base_sort(&input_file, None, &params);
    assert!(explicit_output.status.success(), "gaf-base sort failed with explicit args");
    let correct_output = explicit_output.stdout == truth;
    assert!(correct_output, "gaf-base sort produced incorrect output with explicit args");
    assert!(explicit_output.stderr.is_empty(), "gaf-base sort produced stderr output with explicit args");

    let output_file = temp_files.new_file("sorted");
    let file_output = run_gaf_base_sort(&input_file, Some(&output_file), &params);
    assert!(file_output.status.success(), "gaf-base sort failed with file output");
    assert!(file_output.stdout.is_empty(), "gaf-base sort produced stdout output with file output");
    assert!(file_output.stderr.is_empty(), "gaf-base sort produced stderr output with file output");
    assert!(compare_files(&output_file, &truth_file), "gaf-base sort produced incorrect output with file output");

    let params = SortParameters { progress: true, ..params };
    let progress_output = run_gaf_base_sort(&input_file, None, &params);
    assert!(progress_output.status.success(), "gaf-base sort failed with --progress");
    let correct_output = progress_output.stdout == truth;
    assert!(correct_output, "gaf-base sort produced incorrect output with --progress");
    assert!(!progress_output.stderr.is_empty(), "gaf-base sort did not produce stderr output with --progress");
}

#[test]
fn gaf_base_sort_stable() {
    let mut temp_files = TempFileHandler::new();
    let input_file = utils::get_test_data("shuffled.gaf");
    let params = SortParameters { stable: true, ..SortParameters::default() };

    let truth_file = temp_files.new_file("stable");
    sort_gaf(&input_file, &truth_file, &params).expect("Failed to sort GAF file for truth");
    let truth = fs::read(&truth_file).expect("Failed to read truth file");

    let stable_output = run_gaf_base_sort(&input_file, None, &params);
    assert!(stable_output.status.success(), "gaf-base sort failed with --stable");
    let correct_output = stable_output.stdout == truth;
    assert!(correct_output, "gaf-base sort produced incorrect output with --stable");
    assert!(stable_output.stderr.is_empty(), "gaf-base sort produced stderr output with --stable");
}

#[test]
fn gaf_base_sort_shuffle() {
    let mut temp_files = TempFileHandler::new();
    let input_file = utils::get_test_data("micb-kir3dl1_HG003.gaf");
    let params = SortParameters { key_type: KeyType::Hash, ..SortParameters::default() };

    let truth_file = temp_files.new_file("shuffle");
    sort_gaf(&input_file, &truth_file, &params).expect("Failed to shuffle GAF file for truth");
    let truth = fs::read(&truth_file).expect("Failed to read truth file");

    let shuffled_output = run_gaf_base_sort(&input_file, None, &params);
    assert!(shuffled_output.status.success(), "gaf-base sort failed with --key-type hash");
    let correct_output = shuffled_output.stdout == truth;
    assert!(correct_output, "gaf-base sort produced incorrect output with --key-type hash");
    assert!(shuffled_output.stderr.is_empty(), "gaf-base sort produced stderr output with --key-type hash");
}

fn expect_merge_rounds(input_file: &PathBuf, truth: &[u8], params: &SortParameters, expected_rounds: usize) {
    let output = run_gaf_base_sort(input_file, None, params);
    assert!(output.status.success(), "gaf-base sort failed with {} merge round(s)", expected_rounds);
    let correct_output = output.stdout == truth;
    assert!(correct_output, "gaf-base sort produced incorrect output with {} merge round(s)", expected_rounds);
    let rounds = count_merge_rounds(&output.stderr);
    assert_eq!(rounds, expected_rounds, "wrong number of merge rounds");
}

#[test]
fn gaf_base_sort_params() {
    let mut temp_files = TempFileHandler::new();
    // The input file has 12439 GAF records.
    let input_file = utils::get_test_data("shuffled.gaf");
    let params = SortParameters { stable: true, ..Default::default() };

    // We can reuse the truth, as we use stable sorting.
    let truth_file = temp_files.new_file("sort-params");
    sort_gaf(&input_file, &truth_file, &params).expect("Failed to sort GAF file for truth");
    let truth = fs::read(&truth_file).expect("Failed to read truth file");

    // We didn't want to print progress when sorting the truth.
    let params = SortParameters { progress: true, ..params };

    // Single batch.
    {
        expect_merge_rounds(&input_file, &truth, &params, 0);
    }

    // One round of merges.
    {
        let params = SortParameters { records_per_file: 2000, ..params };
        expect_merge_rounds(&input_file, &truth, &params, 1);
    }

    // Two rounds of merges.
    {
        let params = SortParameters { records_per_file: 2000, files_per_merge: 3, ..params };
        expect_merge_rounds(&input_file, &truth, &params, 2);
    }

    // Multithreaded sort with two rounds of merges.
    {
        let params = SortParameters {
            records_per_file: 2000, files_per_merge: 3, threads: 2, progress: false, ..params
        };
        let output = run_gaf_base_sort(&input_file, None, &params);
        assert!(output.status.success(), "gaf-base sort failed with 2 threads");
        let correct_output = output.stdout == truth;
        assert!(correct_output, "gaf-base sort produced incorrect output with 2 threads");
        assert!(output.stderr.is_empty(), "gaf-base sort produced stderr output with 2 threads");
    }
}

// TODO: presets

//-----------------------------------------------------------------------------

// Tests for `gaf-base compress`.
// NOTE: micb-kir3dl1_HG003.gaf is already sorted with `vg gamsort`, which uses a slightly
// different sort key than `gaf-base sort`. micb-kir3dl1_HG003.gbwt uses the same order.

// We assume that the sorted input file is a temporary file if no output file is provided.
// Otherwise there may be conflicts with multiple tests running in parallel.
fn run_gaf_base_compress(
    temp_files: &mut TempFileHandler,
    sorted_input: &PathBuf, gbwt_file: Option<&PathBuf>, graph_file: Option<&PathBuf>, output_file: Option<&PathBuf>,
    params: &GAFBaseParams, overwrite: bool
) -> (PathBuf, ExitStatus) {
    let mut args = vec![String::from("compress")];
    let output_file = match output_file {
        Some(path) => {
            args.push(String::from("--output"));
            args.push(path.to_str().unwrap().to_string());
            path.clone()
        },
        None => {
            let path = sorted_input.with_added_extension("db");
            temp_files.add_file(&path);
            path
        },
    };
    if let Some(gbwt_file) = gbwt_file {
        args.push(String::from("--gbwt"));
        args.push(gbwt_file.to_str().unwrap().to_string());
    }
    if let Some(graph_file) = graph_file {
        args.push(String::from("--ref-free"));
        args.push(graph_file.to_str().unwrap().to_string());
    }
    args.push(String::from("--block-size"));
    args.push(params.block_size.to_string());
    if !params.store_quality_strings {
        args.push(String::from("--no-quality"));
    }
    if !params.store_optional_fields {
        args.push(String::from("--no-optional"));
    }
    if overwrite {
        args.push(String::from("--overwrite"));
    }
    args.push(sorted_input.to_str().unwrap().to_string());

    let binary = get_binary_path("gaf-base");
    let result = Command::new(binary)
        .args(&args)
        .output()
        .expect("Failed to execute gaf-base compress");
    (output_file, result.status)
}

#[test]
fn gaf_base_compress() {
    // In this test, we also check that we can build GAF-base from the output of
    // `gaf-base sort`. Other tests start from a pre-sorted GAF file.
    let mut temp_files = TempFileHandler::new();
    let original_input = utils::get_test_data("shuffled.gaf");
    let sorted_input = sort_gaf_to_temp(&mut temp_files, &original_input);
    let gbwt_file = None;
    let graph_file = None;
    let params = GAFBaseParams::default();

    // We expect that the GAF-base built with `gaf-base compress` is identical to
    // one built with a direct library call.
    let truth_file = build_gaf_base_truth(
        &mut temp_files, &sorted_input, None, None, &GAFBaseParams::default()
    );
    let (output_file, status) = run_gaf_base_compress(
        &mut temp_files, &sorted_input, gbwt_file, graph_file, None, &params, false
    );
    assert!(status.success(), "gaf-base compress failed with status: {}", status);
    assert!(compare_files(&output_file, &truth_file), "Output file does not match truth file");
    let output_metadata = fs::metadata(&output_file).expect("Failed to read output file metadata");
    let old_timestamp = output_metadata.modified().expect("Failed to get output file timestamp");

    // Try rebuilding the database without overwrite, expect failure.
    let (_, status) = run_gaf_base_compress(
        &mut temp_files, &sorted_input, gbwt_file, graph_file, None, &params, false
    );
    assert!(!status.success(), "gaf-base compress should have failed without overwrite");

    // Rebuild with overwrite, expect success.
    let (output_file, status) = run_gaf_base_compress(
        &mut temp_files, &sorted_input, gbwt_file, graph_file, None, &params, true
    );
    assert!(status.success(), "gaf-base compress failed with overwrite with status: {}", status);

    // Check that we did actually overwrite the output file.
    let output_metadata = fs::metadata(&output_file).expect("Failed to read output file metadata");
    let new_timestamp = output_metadata.modified().expect("Failed to get output file timestamp");
    assert!(new_timestamp > old_timestamp, "Output file timestamp did not change after overwrite");
    assert!(compare_files(&output_file, &truth_file), "Output file does not match truth file after overwrite");

    // Check that the default parameters correspond to the default arguments.
    let input_copy = temp_files.create_copy(&sorted_input);
    let default_args = vec![String::from("compress"), input_copy.to_str().unwrap().to_string()];
    let binary = get_binary_path("gaf-base");
    let default_output = Command::new(&binary)
        .args(&default_args)
        .output()
        .expect("Failed to execute gaf-base compress with default args");
    assert!(default_output.status.success(), "gaf-base compress failed with default args");
    let correct_output = compare_files(&output_file, &truth_file);
    assert!(correct_output, "gaf-base compress produced incorrect output with default args");

    // Try specifying an explicit output file name.
    let explicit_output_file = temp_files.new_file("gaf-base-output");
    let (_, status) = run_gaf_base_compress(
        &mut temp_files, &sorted_input, gbwt_file, graph_file, Some(&explicit_output_file), &params, false
    );
    assert!(status.success(), "gaf-base compress failed with explicit output file with status: {}", status);
    assert!(compare_files(&explicit_output_file, &truth_file), "Explicit output file does not match truth file");
}

#[test]
fn gaf_base_compress_ref_free() {
    let mut temp_files = TempFileHandler::new();
    let sorted_input = utils::get_test_data("micb-kir3dl1_HG003.gaf");
    let gbwt_file = None;
    let graph_file = utils::get_test_data("micb-kir3dl1.gbz");
    let params = GAFBaseParams { reference_free: true, ..GAFBaseParams::default() };

    let truth_file = build_gaf_base_truth(
        &mut temp_files, &sorted_input, gbwt_file, Some(&graph_file), &params
    );
    let output_file = temp_files.new_file("gaf-base");
    let (_, status) = run_gaf_base_compress(
        &mut temp_files, &sorted_input, gbwt_file, Some(&graph_file), Some(&output_file), &params, false
    );
    assert!(status.success(), "gaf-base compress ref-free failed with status: {}", status);
    assert!(compare_files(&output_file, &truth_file), "Output file does not match truth file");
}

#[test]
fn gaf_base_compress_with_gbwt() {
    let mut temp_files = TempFileHandler::new();
    let sorted_input = utils::get_test_data("micb-kir3dl1_HG003.gaf");
    let gbwt_file = utils::get_test_data("micb-kir3dl1_HG003.gbwt");
    let graph_file = None;
    let params = GAFBaseParams::default();

    let truth_file = build_gaf_base_truth(
        &mut temp_files, &sorted_input, Some(&gbwt_file), graph_file, &params
    );
    let output_file = temp_files.new_file("gaf-base");
    let (_, status) = run_gaf_base_compress(
        &mut temp_files, &sorted_input, Some(&gbwt_file), graph_file, Some(&output_file), &params, false
    );
    assert!(status.success(), "gaf-base compress with GBWT failed with status: {}", status);
    assert!(compare_files(&output_file, &truth_file), "Output file does not match truth file");
}

#[test]
fn gaf_base_compress_no_quality() {
    let mut temp_files = TempFileHandler::new();
    let sorted_input = utils::get_test_data("micb-kir3dl1_HG003.gaf");
    let gbwt_file = None;
    let graph_file = None;
    let params = GAFBaseParams { store_quality_strings: false, ..GAFBaseParams::default() };

    let truth_file = build_gaf_base_truth(
        &mut temp_files, &sorted_input, gbwt_file, graph_file, &params
    );
    let output_file = temp_files.new_file("gaf-base");
    let (_, status) = run_gaf_base_compress(
        &mut temp_files, &sorted_input, gbwt_file, graph_file, Some(&output_file), &params, false
    );
    assert!(status.success(), "gaf-base compress no quality failed with status: {}", status);
    assert!(compare_files(&output_file, &truth_file), "Output file does not match truth file");
}

#[test]
fn gaf_base_compress_no_optional() {
    let mut temp_files = TempFileHandler::new();
    let sorted_input = utils::get_test_data("micb-kir3dl1_HG003.gaf");
    let gbwt_file = None;
    let graph_file = None;
    let params = GAFBaseParams { store_optional_fields: false, ..GAFBaseParams::default() };

    let truth_file = build_gaf_base_truth(
        &mut temp_files, &sorted_input, gbwt_file, graph_file, &params
    );
    let output_file = temp_files.new_file("gaf-base");
    let (_, status) = run_gaf_base_compress(
        &mut temp_files, &sorted_input, gbwt_file, graph_file, Some(&output_file), &params, false
    );
    assert!(status.success(), "gaf-base compress no optional failed with status: {}", status);
    assert!(compare_files(&output_file, &truth_file), "Output file does not match truth file");
}

#[test]
fn gaf_base_compress_block_sizes() {
    let mut temp_files = TempFileHandler::new();
    let sorted_input = utils::get_test_data("micb-kir3dl1_HG003.gaf");
    let gbwt_file = None;
    let graph_file = None;
    let block_sizes = vec![10, 100, 1000];

    for block_size in block_sizes {
        let params = GAFBaseParams { block_size, ..GAFBaseParams::default() };
        let truth_file = build_gaf_base_truth(
            &mut temp_files, &sorted_input, gbwt_file, graph_file, &params
        );
        let output_file = temp_files.new_file("gaf-base");
        let (_, status) = run_gaf_base_compress(
            &mut temp_files, &sorted_input, gbwt_file, graph_file, Some(&output_file), &params, false
        );
        assert!(status.success(), "gaf-base compress with block size {} failed with status: {}", block_size, status);
        assert!(compare_files(&output_file, &truth_file), "Output file does not match truth file for block size {}", block_size);
    }
}

// TODO: presets

//-----------------------------------------------------------------------------

// Tests for `gaf-base decompress`.
// There is no corresponding library function, and the output is not guaranteed to be
// identical to the input. (For example, the representation is normalized and the order
// of optional fields may change).

fn run_gaf_base_decompress(
    input_file: &PathBuf, output_file: Option<&PathBuf>, graph_file: Option<&PathBuf>,
    chunk_size: Option<usize>
) -> Output {
    let mut args = vec![String::from("decompress")];
    if let Some(output_file) = output_file {
        args.push(String::from("--output"));
        args.push(output_file.to_str().unwrap().to_string());
    }
    if let Some(graph_file) = graph_file {
        args.push(String::from("--reference"));
        args.push(graph_file.to_str().unwrap().to_string());
    }
    if let Some(chunk_size) = chunk_size {
        args.push(String::from("--chunk-size"));
        args.push(chunk_size.to_string());
    }
    args.push(input_file.to_str().unwrap().to_string());

    let binary = get_binary_path("gaf-base");
    Command::new(binary)
        .args(&args)
        .output()
        .expect("Failed to execute gaf-base decompress")
}

#[test]
fn gaf_base_decompress() {
    let mut temp_files = TempFileHandler::new();
    let sorted_input = utils::get_test_data("micb-kir3dl1_HG003.gaf");
    let gbwt_file = None;
    let graph_file = utils::get_test_data("micb-kir3dl1.gbz");
    let params = GAFBaseParams::default();
    let expected_lines = count_lines_in_file(&sorted_input);
    let input_file = build_gaf_base_truth(&mut temp_files, &sorted_input, gbwt_file, None, &params);
    let output_file = None;
    let chunk_size = None;

    // Default arguments, store the output as the expected baseline.
    let baseline;
    {
        let output = run_gaf_base_decompress(&input_file, output_file, Some(&graph_file), chunk_size);
        assert!(output.status.success(), "gaf-base decompress failed with status: {}", output.status);
        let output_lines = count_lines(&output.stdout);
        assert_eq!(output_lines, expected_lines, "Decompressed output has incorrect number of lines");
        baseline = output.stdout;
    }

    // Try different chunk sizes.
    for chunk_size in [1, 10, 100] {
        let output = run_gaf_base_decompress(&input_file, output_file, Some(&graph_file), Some(chunk_size));
        assert!(output.status.success(), "gaf-base decompress with chunk size {} failed with status: {}", chunk_size, output.status);
        let correct_output = output.stdout == baseline;
        assert!(correct_output, "gaf-base decompress produced unexpected output with chunk size {}", chunk_size);
    }

    // Specify an explicit output file.
    {
        let output_file = temp_files.new_file("decompressed");
        let output = run_gaf_base_decompress(&input_file, Some(&output_file), Some(&graph_file), chunk_size);
        assert!(output.status.success(), "gaf-base decompress with file output failed with status: {}", output.status);
        let output_data = fs::read(&output_file).expect("Failed to read output file");
        let correct_output = output_data == baseline;
        assert!(correct_output, "gaf-base decompress produced unexpected output in file");
    }

    // Build and decompress a reference-free GAF-base.
    {
        let params = GAFBaseParams { reference_free: true, ..GAFBaseParams::default() };
        let input_file = build_gaf_base_truth(&mut temp_files, &sorted_input, gbwt_file, Some(&graph_file), &params);
        let output = run_gaf_base_decompress(&input_file, output_file, None, chunk_size);
        assert!(output.status.success(), "reference-free gaf-base decompress failed with status: {}", output.status);
        let correct_output = output.stdout == baseline;
        assert!(correct_output, "gaf-base decompress produced unexpected output for reference-free GAF-base");
    }
}

//-----------------------------------------------------------------------------

// Tests for `gbz-base query`.

// nodes, handles, offsets, intervals, between

// context length, snarls, extend snarls

// safety limit

// distinct, ref-only

// cigar

// JSON output

// GAF output: overlapping, clipped, contained

//-----------------------------------------------------------------------------
