// SPDX-FileCopyrightText: 2026 Frederic Ruget <fred@atlant.is> (GitHub: @douzebis)
//
// SPDX-License-Identifier: MIT

use std::ffi::OsStr;
use std::fs::File;
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::Path;

use base91::io::{DecoderReader, EncoderWriter};
use clap::{CommandFactory, Parser};
use clap_complete::{generate, Shell};

// ---------------------------------------------------------------------------
// CountingWriter
// ---------------------------------------------------------------------------

struct CountingWriter<'a> {
    inner: &'a mut dyn Write,
    count: usize,
}

impl Write for CountingWriter<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let n = self.inner.write(buf)?;
        self.count += n;
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

// ---------------------------------------------------------------------------
// CLI definition
// ---------------------------------------------------------------------------

/// basE91 encode or decode FILE, or standard input, to standard output.
///
/// With no FILE, or when FILE is -, read standard input.
#[derive(Parser, Debug)]
#[command(
    name    = "base91",
    version = env!("CARGO_PKG_VERSION"),
    author  = "Joachim Henke (algorithm); Frederic Ruget (Rust port)",
    after_help = "\
Examples:
  base91 file.bin > file.b91            encode binary file
  base91 -d file.b91 > file.bin         decode
  b91enc < file.bin > file.b91          encode with no line wrapping
  b91dec < file.b91 > file.bin          decode
  base91 --simd file.bin > file.b91s    encode with SIMD variant
  tar czf - dir/ | b91enc               encode tar stream"
)]
struct Cli {
    /// Decode data instead of encoding.
    #[arg(short = 'd', long)]
    decode: bool,

    /// Encode data (explicit override of --decode).
    #[arg(short = 'e', hide = true)]
    encode: bool,

    /// Write output to FILE instead of standard output.
    #[arg(short = 'o', long, value_name = "FILE")]
    output: Option<String>,

    /// Use SIZE bytes of memory for I/O buffers (suffixes: b, K, M).
    /// Default: 64K.
    #[arg(short = 'm', long = "buffer", value_name = "SIZE",
          value_parser = parse_size)]
    buffer: Option<usize>,

    /// Verbose mode: print statistics to stderr.
    /// Repeat (-vv) for extra verbosity.
    #[arg(short = 'v', long, action = clap::ArgAction::Count)]
    verbose: u8,

    /// Wrap encoded lines after COLS characters (0 = no wrap; default 76).
    /// Has no effect when decoding.
    #[arg(short = 'w', long, value_name = "COLS")]
    wrap: Option<usize>,

    /// Use the SIMD fixed-width variant for encoding.
    ///
    /// Output begins with '-' and uses the SIMD alphabet (0x23-0x26,
    /// 0x28-0x7E); not compatible with legacy Henke decoders. Output
    /// contains no single-quote characters and is safe to single-quote
    /// in shell. Ignored when decoding. With --wrap, the value must be
    /// a multiple of 32.
    #[arg(long)]
    simd: bool,

    /// Generate shell completions for SHELL and exit.
    #[arg(long, value_name = "SHELL", hide = true)]
    completions: Option<Shell>,

    /// Input file (default: stdin).
    #[arg(value_name = "FILE")]
    file: Option<String>,
}

fn parse_size(s: &str) -> Result<usize, String> {
    let (digits, suffix) = s.split_at(s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len()));
    if digits.is_empty() {
        return Err(format!("invalid SIZE argument: `{s}'"));
    }
    let n: usize = digits
        .parse()
        .map_err(|_| format!("invalid SIZE argument: `{s}'"))?;
    let mult = match suffix.to_lowercase().as_str() {
        "" | "b" => 1,
        "k" => 1024,
        "m" => 1024 * 1024,
        _ => return Err(format!("invalid SIZE suffix: `{suffix}'")),
    };
    Ok(n * mult)
}

// ---------------------------------------------------------------------------
// Invocation name detection (b91enc / b91dec behaviour)
// ---------------------------------------------------------------------------

/// Returns `(decode_default, wrap_default)` based on argv[0].
///
/// - `b91enc`: encode, no line wrapping
/// - `b91dec`: decode
/// - anything else (`base91`): encode, wrap at 76
fn defaults_from_progname(argv0: &OsStr) -> (bool, usize) {
    let name = Path::new(argv0)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("base91");
    // Strip a leading "lt-" that libtool sometimes prepends in test runs.
    let name = name.strip_prefix("lt-").unwrap_or(name);
    match name {
        "b91enc" => (false, 0), // encode, no wrap
        "b91dec" => (true, 0),  // decode
        _ => (false, 76),       // base91: encode, wrap=76
    }
}

// ---------------------------------------------------------------------------
// I/O helpers
// ---------------------------------------------------------------------------

fn open_input(file: Option<&str>) -> io::Result<Box<dyn Read>> {
    match file {
        None | Some("-") => Ok(Box::new(io::stdin().lock())),
        Some(path) => Ok(Box::new(BufReader::new(File::open(path)?))),
    }
}

fn open_output(file: Option<&str>) -> io::Result<Box<dyn Write>> {
    match file {
        None => Ok(Box::new(BufWriter::new(io::stdout().lock()))),
        Some("-") => Ok(Box::new(BufWriter::new(io::stdout().lock()))),
        Some(path) => Ok(Box::new(BufWriter::new(File::create(path)?))),
    }
}

// ---------------------------------------------------------------------------
// Encode path
// ---------------------------------------------------------------------------

fn do_encode(
    input: &mut dyn Read,
    output: &mut dyn Write,
    buf_size: usize,
    wrap: usize,
) -> io::Result<(usize, usize)> {
    let ibuf_size = compute_ibuf_encode(buf_size);
    let mut ibuf = vec![0u8; ibuf_size];
    let mut itotal: usize = 0;
    let ototal;

    if wrap == 0 {
        // No wrapping: feed directly through EncoderWriter.
        // Wrap output in a byte counter so we can report the exact ratio.
        let mut counter = CountingWriter {
            inner: output,
            count: 0,
        };
        let mut enc = EncoderWriter::new(&mut counter);
        loop {
            let n = input.read(&mut ibuf)?;
            if n == 0 {
                break;
            }
            itotal += n;
            enc.write_all(&ibuf[..n])?;
        }
        enc.finish()?;
        ototal = counter.count;
    } else {
        // With wrapping: encode to a temporary buffer, then wrap-write.
        let mut enc = EncoderWriter::new(Vec::<u8>::new());
        loop {
            let n = input.read(&mut ibuf)?;
            if n == 0 {
                break;
            }
            itotal += n;
            enc.write_all(&ibuf[..n])?;
        }
        let encoded: Vec<u8> = enc.finish()?;
        ototal = encoded.len();

        // Write with line wrapping.
        let mut col = 0usize;
        for &byte in &encoded {
            output.write_all(&[byte])?;
            col += 1;
            if col == wrap {
                output.write_all(b"\n")?;
                col = 0;
            }
        }
        if col > 0 {
            output.write_all(b"\n")?;
        }
    }

    Ok((itotal, ototal))
}

fn do_encode_simd(
    input: &mut dyn Read,
    output: &mut dyn Write,
    buf_size: usize,
    wrap: usize,
) -> io::Result<(usize, usize)> {
    let ibuf_size = compute_ibuf_encode(buf_size);
    let mut ibuf = vec![0u8; ibuf_size];
    let mut itotal: usize = usize::default();

    // Buffer the entire input, then encode in one shot.
    let mut raw: Vec<u8> = Vec::new();
    loop {
        let n = input.read(&mut ibuf)?;
        if n == 0 {
            break;
        }
        itotal += n;
        raw.extend_from_slice(&ibuf[..n]);
    }
    let encoded = base91::simd::encode(&raw, base91::simd::SimdLevel::default(), wrap);

    let ototal = encoded.len();
    output.write_all(&encoded)?;
    Ok((itotal, ototal))
}

fn compute_ibuf_encode(buf_size: usize) -> usize {
    // Mirror the C CLI arithmetic: ibuf_size = (buf_size - 2) * 16 / 29
    let n = (buf_size.saturating_sub(2)) * 16 / 29;
    n.max(1)
}

// ---------------------------------------------------------------------------
// Decode path
// ---------------------------------------------------------------------------

fn do_decode_simd(
    input: &mut dyn Read,
    output: &mut dyn Write,
    buf_size: usize,
) -> io::Result<usize> {
    let ibuf_size = buf_size.max(1);
    let mut ibuf = vec![0u8; ibuf_size];
    let mut raw: Vec<u8> = Vec::new();
    loop {
        let n = input.read(&mut ibuf)?;
        if n == 0 {
            break;
        }
        raw.extend_from_slice(&ibuf[..n]);
    }
    let decoded =
        base91::simd::decode(&raw, base91::simd::SimdLevel::default()).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "not a SIMD stream (missing '-' prefix)",
            )
        })?;
    let total = decoded.len();
    output.write_all(&decoded)?;
    Ok(total)
}

fn do_decode(input: &mut dyn Read, output: &mut dyn Write, buf_size: usize) -> io::Result<usize> {
    let ibuf_size = compute_ibuf_decode(buf_size);
    let mut ibuf = vec![0u8; ibuf_size];
    let mut dec = DecoderReader::new(&mut *input);
    let mut total: usize = 0;
    loop {
        let n = dec.read(&mut ibuf)?;
        if n == 0 {
            break;
        }
        total += n;
        output.write_all(&ibuf[..n])?;
    }
    Ok(total)
}

fn compute_ibuf_decode(buf_size: usize) -> usize {
    // Mirror the C CLI arithmetic: ibuf_size = (buf_size - 1) * 8 / 15
    let n = (buf_size.saturating_sub(1)) * 8 / 15;
    n.max(1)
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

fn main() {
    if let Err(e) = run() {
        eprintln!("base91: {e}");
        std::process::exit(1);
    }
}

fn run() -> io::Result<()> {
    let args: Vec<_> = std::env::args_os().collect();
    let (decode_default, wrap_default) = defaults_from_progname(
        args.first()
            .map(|s| s.as_os_str())
            .unwrap_or(OsStr::new("base91")),
    );

    let cli = Cli::parse();

    // Handle --completions early.
    if let Some(shell) = cli.completions {
        let mut cmd = Cli::command();
        generate(shell, &mut cmd, "base91", &mut io::stdout());
        return Ok(());
    }

    let decode = if cli.encode {
        false
    } else if cli.decode {
        true
    } else {
        decode_default
    };

    let simd = cli.simd;
    let buf_size = cli.buffer.unwrap_or(65536);
    // For --simd encoding, replace the Henke wrap default (76, not a multiple
    // of 32) with 64.
    let effective_wrap_default = if simd && !decode { 64 } else { wrap_default };
    let wrap = cli
        .wrap
        .unwrap_or(if decode { 0 } else { effective_wrap_default });
    let verbose = cli.verbose;

    // Validate explicit --wrap multiple-of-32 constraint for --simd (encoding only).
    if simd && !decode && wrap != 0 && wrap % 32 != 0 {
        eprintln!("base91: --wrap value must be a multiple of 32 when --simd is active");
        std::process::exit(1);
    }

    // Validate buffer size.
    if decode {
        let ibuf = compute_ibuf_decode(buf_size);
        if ibuf < 1 {
            eprintln!("base91: SIZE must be >= 3 for decoding");
            std::process::exit(1);
        }
    } else {
        let ibuf = compute_ibuf_encode(buf_size);
        if ibuf < 1 {
            eprintln!("base91: SIZE must be >= 4 for encoding");
            std::process::exit(1);
        }
    }

    let ifile_name = cli.file.as_deref().unwrap_or("standard input");
    let ofile = cli.output.as_deref();

    if verbose >= 2 {
        let ibuf = if decode {
            compute_ibuf_decode(buf_size)
        } else {
            compute_ibuf_encode(buf_size)
        };
        eprintln!("using {buf_size} bytes for buffers; input buffer: {ibuf} bytes");
    }

    if verbose >= 1 {
        let verb = if decode { "decoding" } else { "encoding" };
        eprint!("{verb} {ifile_name} ...");
    }

    let mut input = open_input(cli.file.as_deref())?;
    let mut output = open_output(ofile)?;

    if decode && simd {
        let ototal = do_decode_simd(&mut *input, &mut *output, buf_size)?;
        output.flush()?;
        if verbose >= 1 {
            eprintln!("\tdone ({ototal} bytes, SIMD variant)");
        }
    } else if decode {
        let ototal = do_decode(&mut *input, &mut *output, buf_size)?;
        output.flush()?;
        if verbose >= 1 {
            eprintln!("\tdone ({ototal} bytes)");
        }
    } else if simd {
        let (itotal, _ototal) = do_encode_simd(&mut *input, &mut *output, buf_size, wrap)?;
        output.flush()?;
        if verbose >= 1 {
            eprintln!("\t{itotal} bytes encoded (SIMD variant)");
        }
    } else {
        let (itotal, _ototal) = do_encode(&mut *input, &mut *output, buf_size, wrap)?;
        output.flush()?;
        if verbose >= 1 {
            eprintln!("\t{itotal} bytes encoded");
        }
    }

    Ok(())
}
