use gbz_base::{GBZBase, GraphInterface, GraphReference, GAFBase};
use gbz_base::{Subgraph, ReadSet};
use gbz_base::{SubgraphQuery, HaplotypeOutput, SnarlOutput, AlignmentOutput};
use gbz_base::utils;
use gbz_base::Error;

use gbz::FullPathName;

use simple_sds::binaries;

use std::io::BufRead;
use std::time::Instant;
use std::{env, process};

use getopts::Options;

use rand::{Rng, SeedableRng};

//-----------------------------------------------------------------------------

fn main() -> Result<(), Error> {
    let start_time = Instant::now();

    let config = Config::new().map_err(Error::invalid_query)?;
    let queries = generate_queries(&config).map_err(Error::invalid_query)?;

    // Open GBZ-base and GAF-base.
    let gbz_base = GBZBase::open(&config.gbz_base_file)?;
    let mut graph = GraphInterface::new(&gbz_base)?;
    let gaf_base = if let Some(gaf_base_file) = &config.gaf_base_file {
        Some(GAFBase::open(gaf_base_file)?)
    } else {
        None
    };

    // Run queries and report results.
    let mut results = Vec::new();
    for query in queries.iter() {
        let query_start = Instant::now();
        let mut result = QueryResult::default();

        let mut subgraph = Subgraph::new();
        subgraph.from_db(&mut graph, query)?;
        result.nodes = subgraph.nodes();
        if let Some(gaf_base) = &gaf_base {
            let graph_ref = GraphReference::Db(&mut graph);
            let read_set = ReadSet::new(graph_ref, &subgraph, gaf_base, AlignmentOutput::Clipped)?;
            result.fragments = read_set.len();
            result.alignments = read_set.unclipped();
            result.blocks = read_set.blocks();
            result.candidates = read_set.candidates();
            result.handles = read_set.node_records();
        }

        let query_end = Instant::now();
        result.time = query_end.duration_since(query_start).as_secs_f64();
        if config.verbose {
            result.print(query);
        }
        results.push(result);
    }
    report_results(&queries, &results, &config);

    let end_time = Instant::now();
    let seconds = end_time.duration_since(start_time).as_secs_f64();
    eprintln!("Used {:.3} seconds", seconds);
    utils::report_peak_memory_usage();

    Ok(())
}

//-----------------------------------------------------------------------------

struct Config {
    // Input files.
    gbz_base_file: String,
    gaf_base_file: Option<String>,
    faidx_file: String,

    // Query parameters.
    sample_name: String,
    interval_length: usize,
    num_queries: usize,
    context_length: usize,
    snarl_output: SnarlOutput,
    seed: u64,

    // Output options.
    verbose: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            gbz_base_file: String::new(),
            gaf_base_file: None,
            faidx_file: String::new(),
            sample_name: String::new(),
            interval_length: Self::DEFAULT_INTERVAL_LENGTH,
            num_queries: Self::DEFAULT_NUM_QUERIES,
            context_length: Self::DEFAULT_CONTEXT_LENGTH,
            snarl_output: Self::DEFAULT_SNARL_OUTPUT,
            seed: Self::DEFAULT_SEED,
            verbose: false,
        }
    }
}

impl Config {
    const DEFAULT_INTERVAL_LENGTH: usize = 100;
    const DEFAULT_NUM_QUERIES: usize = 10000;
    const DEFAULT_CONTEXT_LENGTH: usize = 100;
    const DEFAULT_SNARL_OUTPUT: SnarlOutput = SnarlOutput::None;
    const DEFAULT_SEED: u64 = 42;

    fn new() -> Result<Self, String> {
        let mut config = Config::default();

        let args: Vec<String> = env::args().collect();
        let program = args[0].clone();
        let header = format!("Usage: {} [options]", program);

        let mut opts = Options::new();
        opts.optflag("h", "help", "print this help");
        opts.reqopt("", "gbz-base", "GBZ-base file for the graph (required)", "FILE");
        opts.optopt("", "gaf-base", "GAF-base file for the alignments (optional)", "FILE");
        opts.reqopt("", "faidx", "faidx file for the reference (required)", "FILE");
        opts.reqopt("", "sample", "sample name for reference paths (required)", "NAME");
        opts.optopt("", "interval-length", &format!("length of query intervals in bp (default: {})", Self::DEFAULT_INTERVAL_LENGTH), "INT");
        opts.optopt("", "num-queries", &format!("number of queries to run (default: {})", Self::DEFAULT_NUM_QUERIES), "INT");
        opts.optopt("", "greedy-context", &format!("extract context (in bp) around the query interval (default: {})", Self::DEFAULT_CONTEXT_LENGTH), "INT");
        opts.optflag("", "snarls", "extract top-level snarls contained in the query interval");
        opts.optopt("", "seed", &format!("random seed for query generation (default: {})", Self::DEFAULT_SEED), "INT");
        opts.optflag("", "verbose", "print query-level statistics to stdout");
        let matches = match opts.parse(&args[1..]) {
            Ok(m) => m,
            Err(f) => {
                eprintln!("{}", opts.usage(&header));
                // We need this hack, because the presence of required options is checked before we
                // get a chance to print help.
                if args_contain_help() {
                    process::exit(0);
                }
                return Err(format!("{}", f));
            }
        };

        config.gbz_base_file = matches.opt_str("gbz-base").unwrap();
        config.gaf_base_file = matches.opt_str("gaf-base");
        config.faidx_file = matches.opt_str("faidx").unwrap();

        config.sample_name = matches.opt_str("sample").unwrap();
        if let Some(s) = matches.opt_str("interval-length") {
            config.interval_length = parse_large_quantity(&s, "--interval-length")?;
        }
        if let Some(s) = matches.opt_str("num-queries") {
            config.num_queries = parse_large_quantity(&s, "--num-queries")?;
        }
        if let Some(s) = matches.opt_str("greedy-context") {
            config.context_length = parse_large_quantity(&s, "--greedy-context")?;
        }
        if matches.opt_present("snarls") {
            config.snarl_output = SnarlOutput::Contained;
        }
        // We do not support --extend-snarls, as the size of the output subgraph
        // is unpredictable in graphs with large snarls.

        // We want to generate the same queries for different GAF-base / context / snarl options,
        // but different queries if the interval length or the number of queries changes.
        // (As currently implemented, changing interval length will change the queries anyway.)
        if let Some(s) = matches.opt_str("seed") {
            config.seed = parse_integer(&s, "--seed")? as u64;
        }
        config.seed ^= config.interval_length as u64;
        config.seed ^= config.num_queries as u64;

        config.verbose = matches.opt_present("verbose");

        Ok(config)
    }
}

//-----------------------------------------------------------------------------

fn args_contain_help() -> bool {
    for arg in env::args() {
        if arg == "-h" || arg == "--help" {
            return true;
        }
    }
    false
}

fn parse_integer(s: &str, description: &str) -> Result<usize, String> {
    s.parse::<usize>().map_err(|x| format!("Failed to parse {}: {}", description, x))
}

fn parse_large_quantity(s: &str, description: &str) -> Result<usize, String> {
    binaries::parse_unsigned(s).map_err(|x| format!("Failed to parse {}: {}", description, x))
}

// Parses a faidx file. Returns a vector of (contig name, contig length - query interval length + 1)
// pairs for contigs of length at least the query interval length.
// The second element of the pair is the number of possible query intervals in the contig.
fn parse_faidx(faidx_file: &str, interval_length: usize) -> Result<Vec<(String, usize)>, String> {
    let mut contigs = Vec::new();
    let file = std::fs::File::open(faidx_file).map_err(|x| format!("Failed to open faidx file {}: {}", faidx_file, x))?;
    let reader = std::io::BufReader::new(file);
    for line in reader.lines() {
        let line = line.map_err(|x| format!("Failed to read line from faidx file {}: {}", faidx_file, x))?;
        let parts: Vec<&str> = line.split('\t').collect();
        if parts.len() < 2 {
            return Err(format!("Invalid line in faidx file {}: {}", faidx_file, line));
        }

        // We may have a PanSN path name as a contig name.
        let contig_parts: Vec<&str> = parts[0].split('#').collect();
        let contig_name = if contig_parts.len() == 3 {
            contig_parts[2]
        } else {
            parts[0]
        }.to_string();

        let contig_length = parse_integer(parts[1], "contig length")?;
        if contig_length >= interval_length {
            contigs.push((contig_name, contig_length - interval_length + 1));
        }
    }
    Ok(contigs)
}

fn generate_queries(config: &Config) -> Result<Vec<SubgraphQuery>, String> {
    let contigs = parse_faidx(&config.faidx_file, config.interval_length)?;
    if contigs.is_empty() {
        return Err(format!("No contigs in faidx file {} are long enough for the query interval length {}", config.faidx_file, config.interval_length));
    }
    let total_length = contigs.iter().map(|(_, len)| *len).sum::<usize>();

    let mut queries = Vec::new();
    let mut rng = rand::rngs::StdRng::seed_from_u64(config.seed);
    for _ in 0..config.num_queries {
        let mut offset = rng.random_range(0..total_length);
        let mut contig_index = 0;
        while offset >= contigs[contig_index].1 {
            offset -= contigs[contig_index].1;
            contig_index += 1;
        }

        let path_name = FullPathName::reference(&config.sample_name, &contigs[contig_index].0);
        let query = SubgraphQuery::path_interval(&path_name, offset..(offset + config.interval_length))
            .with_context(config.context_length)
            .with_snarls(config.snarl_output)
            .with_haplotypes(HaplotypeOutput::Distinct);
        queries.push(query);
    }

    Ok(queries)
}

//-----------------------------------------------------------------------------

#[derive(Default)]
struct QueryResult {
    nodes: usize,
    fragments: usize,
    alignments: usize,
    blocks: usize,
    candidates: usize,
    handles: usize,
    time: f64,
}

impl QueryResult {
    // Prints the result in a tab-delimited format to stdout.
    fn print(&self, query: &SubgraphQuery) {
        println!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{:.6}",
            query, self.nodes,
            self.fragments, self.alignments,
            self.blocks, self.candidates,
            self.handles,
            self.time
        );
    }
}

fn report_results(queries: &[SubgraphQuery], results: &[QueryResult], config: &Config) {
    let divisor = if results.is_empty() { 1.0 } else { results.len() as f64 };
    eprintln!(
        "{} queries with interval length {} bp, context length {} bp, and snarl output '{}'",
        queries.len(), config.interval_length, config.context_length, config.snarl_output
    );
    eprintln!();

    eprintln!("Averages:");
    eprintln!();
    let total_nodes: f64 = results.iter().map(|r| r.nodes as f64).sum();
    eprintln!("Subgraph nodes: {:.3}", total_nodes / divisor);
    if config.gaf_base_file.is_some() {
        let total_fragments: f64 = results.iter().map(|r| r.fragments as f64).sum();
        eprintln!("Fragments: {:.3}", total_fragments / divisor);
        let total_alignments: f64 = results.iter().map(|r| r.alignments as f64).sum();
        eprintln!("Alignments: {:.3}", total_alignments / divisor);
        let total_blocks: f64 = results.iter().map(|r| r.blocks as f64).sum();
        eprintln!("Alignment blocks: {:.3}", total_blocks / divisor);
        let total_candidates: f64 = results.iter().map(|r| r.candidates as f64).sum();
        eprintln!("Alignment candidates: {:.3}", total_candidates / divisor);
        let total_handles: f64 = results.iter().map(|r| r.handles as f64).sum();
        eprintln!("Handles: {:.3}", total_handles / divisor);
    }
    let total_time: f64 = results.iter().map(|r| r.time).sum();
    eprintln!("Query time: {:.6} seconds", total_time / divisor);
    eprintln!();
}

//-----------------------------------------------------------------------------
