//! Integration tests for binaries.

use std::fs;
use std::path::PathBuf;
use std::process::{Command, ExitStatus};

use gbz_base::GBZBase;
use gbz_base::utils;

use simple_sds::serialize;

//-----------------------------------------------------------------------------

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

//-----------------------------------------------------------------------------

// Tests for `gbz-base construct`.

// We assume that the graph file is a temporary file if no output file is provided.
// Otherwise there may be conflicts with multiple tests running in parallel.
fn run_gbz_base_construct(
    temp_files: &mut TempFileHandler,
    graph_file: &PathBuf, chains_file: Option<&PathBuf>,
    overwrite: bool, output_file: Option<PathBuf>
) -> (PathBuf, ExitStatus) {
    let mut args = vec![String::from("construct")];
    let output_file = match output_file {
        Some(path) => {
            args.push(String::from("-o"));
            args.push(path.to_str().unwrap().to_string());
            path
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
        &mut temp_files, &graph_file, chains_file, false, Some(explicit_output_file.clone())
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

// gaf-base sort

// basic sort with/without progress

// stable sort, shuffle

// threads

// adjust records per file, merge width

//-----------------------------------------------------------------------------

// gaf-base compress + decompress

// basic compression + decompression

// compression: overwrite, output file

// compression: ref-free, no quality, no optional

// compression with varying block sizes

// decompression with varying chunk sizes

// decompression to a given file

//-----------------------------------------------------------------------------

// gbz-base query

// nodes, handles, offsets, intervals, between

// context length, snarls, extend snarls

// safety limit

// distinct, ref-only

// cigar

// JSON output

// GAF output: overlapping, clipped, contained

//-----------------------------------------------------------------------------
