use gbz_base::{Alignment, GAFBaseParams};
use gbz_base::{formats, utils};

use htscodecs_wrapper::RANSFlags;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use std::{env, process};

use getopts::Options;

//-----------------------------------------------------------------------------

fn main() -> Result<(), String> {
    let start_time = Instant::now();

    let config = Config::new();

    let mut gaf_file = utils::open_file(&config.gaf_file)?;
    let mut line_num: usize = 0; // 0-based, excluding header lines.
    let mut state = State::new();
    loop {
        let mut buf: Vec<u8> = Vec::new();
        let len = gaf_file.read_until(b'\n', &mut buf).map_err(|x| x.to_string())?;
        if len == 0 {
            // End of file.
            break;
        }
        if formats::is_gaf_header_line(&buf) {
            continue;
        }
        let aln = Alignment::from_gaf(&buf).map_err(
            |x| format!("Failed to parse the alignment on line {}: {}", line_num, x)
        )?;
        if aln.is_unaligned() != state.unaligned_block {
            // We have a new block.
            state.flush()?;
            state.unaligned_block = aln.is_unaligned();
        }
        state.add_alignment(&aln);
        if state.current_block_size >= config.params.block_size {
            state.flush()?;
        }
        line_num += 1;
    }
    state.flush()?;

    for (level, size) in state.zstd.iter() {
        let size = utils::human_readable_size(*size);
        println!("zstd -{}: {}", level, size);
    }
    for (flags, size) in state.codecs.iter() {
        let size = utils::human_readable_size(*size);
        println!("{}: {}", flags, size);
    }

    eprintln!(
        "{} alignments of total length {} in {} blocks",
        state.total_alignments, utils::human_readable_size(state.total_length), state.total_blocks
    );
    let end_time = Instant::now();
    let seconds = end_time.duration_since(start_time).as_secs_f64();
    eprintln!("Used {:.3} seconds", seconds);
    utils::report_peak_memory_usage();

    Ok(())
}

//-----------------------------------------------------------------------------

struct State {
    zstd: Vec<(i32, usize)>, // (compression level, total compressed size)
    codecs: Vec<(RANSFlags, usize)>, // (flags, total compressed size)
    buffer: Vec<u8>,
    current_block_size: usize,
    unaligned_block: bool,
    total_alignments: usize,
    total_length: usize,
    total_blocks: usize,
}

impl State {
    fn new() -> Self {
        let zstd = vec![
            (1, 0),
            (3, 0),
            (5, 0),
            (7, 0),
            (9, 0),
        ];
        let codecs = vec![
            (RANSFlags::zero_order(), 0),
            (RANSFlags::zero_order().with_rle(), 0),
            (RANSFlags::zero_order().with_pack(), 0),
            (RANSFlags::zero_order().with_rle().with_pack(), 0),
            (RANSFlags::first_order(), 0),
            (RANSFlags::first_order().with_rle(), 0),
            (RANSFlags::first_order().with_pack(), 0),
            (RANSFlags::first_order().with_rle().with_pack(), 0),
        ];

        State {
            zstd,
            codecs,
            buffer: Vec::new(),
            current_block_size: 0,
            unaligned_block: false,
            total_alignments: 0,
            total_length: 0,
            total_blocks: 0,
        }
    }

    fn add_alignment(&mut self, aln: &Alignment) {
        self.buffer.extend_from_slice(&aln.base_quality);
        self.current_block_size += 1;
        self.total_length += aln.base_quality.len();
        self.total_alignments += 1;
    }

    // TODO: Use rayon?
    fn flush(&mut self) -> Result<(), String> {
        if self.current_block_size == 0 {
            return Ok(());
        }
        self.total_blocks += 1;
        self.current_block_size = 0;
        if self.buffer.is_empty() {
            return Ok(());
        }

        let buffer = Arc::new(self.buffer.clone());

        let zstd_threads = self.zstd.iter().map(|(level, _)| {
            let buffer = Arc::clone(&buffer);
            let level = *level;
            std::thread::spawn(move || zstd_compress(buffer, level))
        }).collect::<Vec<_>>();
        for (i, thread) in zstd_threads.into_iter().enumerate() {
            let compressed_size = thread.join().map_err(|_| String::from("Zstandard compression thread panicked"))?;
            self.zstd[i].1 += compressed_size;
        }

        let rans_threads = self.codecs.iter().map(|(flags, _)| {
            let buffer = Arc::clone(&buffer);
            let flags = *flags;
            std::thread::spawn(move || rans_compress(buffer, flags))
        }).collect::<Vec<_>>();
        for (i, thread) in rans_threads.into_iter().enumerate() {
            let compressed_size = thread.join().map_err(|_| String::from("rANS compression thread panicked"))?;
            self.codecs[i].1 += compressed_size;
        }

        self.buffer.clear();
        Ok(())
    }
}

fn zstd_compress(buffer: Arc<Vec<u8>>, level: i32) -> usize {
    let compressed = zstd::encode_all(&buffer[..], level);
    if compressed.is_err() {
        panic!("Zstandard compression failed for level {}: {}", level, compressed.err().unwrap());
    }
    compressed.unwrap().len()
}

fn rans_compress(buffer: Arc<Vec<u8>>, flags: RANSFlags) -> usize {
    let compressed = htscodecs_wrapper::rans_compress(&buffer, flags);
    if compressed.is_err() {
        panic!("rANS compression failed for flags {}: {}", flags, compressed.err().unwrap());
    }
    compressed.unwrap().len()
}

//-----------------------------------------------------------------------------

struct Config {
    gaf_file: PathBuf,
    params: GAFBaseParams,
}

impl Config {
    fn new() -> Self {
        let mut params = GAFBaseParams::default();

        let args: Vec<String> = env::args().collect();
        let program = args[0].clone();
        let header = format!("Usage: {} [options] alignments.gaf[.gz]", program);

        let mut opts = Options::new();
        opts.optflag("h", "help", "print this help");
        let block_desc = format!("number of alignments per block (default: {})", params.block_size);
        opts.optopt("b", "block-size", &block_desc, "INT");
        let matches = match opts.parse(&args[1..]) {
            Ok(m) => m,
            Err(f) => {
                eprintln!("{}", f);
                process::exit(1);
            }
        };

        if matches.opt_present("h") {
            eprint!("{}", opts.usage(&header));
            process::exit(0);
        }

        let gaf_file = if let Some(s) = matches.free.first() {
            PathBuf::from(s)
        } else {
            eprint!("{}", opts.usage(&header));
            process::exit(1);
        };

        // Parameters.
        if let Some(s) = matches.opt_str("b") {
            match s.parse::<usize>() {
                Ok(size) => params.block_size = size,
                Err(_) => {
                    eprintln!("Invalid block size: {}", s);
                    process::exit(1);
                }
            }
        }

        Config { gaf_file, params }
    }
}

//-----------------------------------------------------------------------------
