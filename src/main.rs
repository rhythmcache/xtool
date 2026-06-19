use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use clap::{Parser, Subcommand};
use data_encoding::BASE32;
use memmap2::Mmap;
use sha2::{Digest, Sha256, Sha512};
use std::fmt::Write as _;
use std::fs::File;
use std::io::{self, Cursor, Read, Write};
use std::path::{Path, PathBuf};

const CHUNK_SIZE: usize = 64 * 1024;

// input

/// opens a streaming input source. regular files are mmap'd and wrapped in a
/// Cursor (avoids a read()-syscall copy n the rest of the pipeline still
/// processes it in chunks via the Read impl). "-" or no path means stdin
/// read in bounded chunks rather than read_to_end.
fn open_input(path: Option<&PathBuf>) -> io::Result<Box<dyn Read>> {
    match path {
        Some(p) if p.as_os_str() != "-" => {
            let file = File::open(p)?;
            match unsafe { Mmap::map(&file) } {
                Ok(mmap) => Ok(Box::new(Cursor::new(mmap))),
                Err(_) => Ok(Box::new(file)),
            }
        }
        _ => Ok(Box::new(io::stdin())),
    }
}

fn stdout_writer() -> io::BufWriter<io::Stdout> {
    io::BufWriter::new(io::stdout())
}

// streaming codec modes

mod modes {
    use super::*;

    //sha256sum / sha512sum

    pub fn sha256_run(mut reader: impl Read, filename: Option<&str>) -> io::Result<()> {
        let mut hasher = Sha256::new();
        let mut buf = [0u8; CHUNK_SIZE];
        loop {
            let n = reader.read(&mut buf)?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }
        print_digest(&hasher.finalize(), filename)
    }

    pub fn sha512_run(mut reader: impl Read, filename: Option<&str>) -> io::Result<()> {
        let mut hasher = Sha512::new();
        let mut buf = [0u8; CHUNK_SIZE];
        loop {
            let n = reader.read(&mut buf)?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }
        print_digest(&hasher.finalize(), filename)
    }

    fn print_digest(digest: &[u8], filename: Option<&str>) -> io::Result<()> {
        let hex = faster_hex::hex_string(digest);
        match filename {
            Some(name) => println!("{hex}  {name}"),
            None => println!("{hex}  -"),
        }
        Ok(())
    }

    // binary (no grouping dependency, streams trivially)

    pub fn binary_encode(mut reader: impl Read, mut writer: impl Write) -> io::Result<()> {
        let mut buf = [0u8; CHUNK_SIZE];
        loop {
            let n = reader.read(&mut buf)?;
            if n == 0 {
                break;
            }
            let mut out = String::with_capacity(n * 8);
            for byte in &buf[..n] {
                let _ = write!(&mut out, "{byte:08b}");
            }
            writer.write_all(out.as_bytes())?;
        }
        writer.write_all(b"\n")?;
        Ok(())
    }

    pub fn binary_decode(mut reader: impl Read, mut writer: impl Write) -> io::Result<()> {
        let mut leftover: Vec<u8> = Vec::new();
        let mut buf = [0u8; CHUNK_SIZE];
        loop {
            let n = reader.read(&mut buf)?;
            if n == 0 {
                break;
            }
            leftover.extend(
                buf[..n]
                    .iter()
                    .copied()
                    .filter(|b| *b == b'0' || *b == b'1'),
            );
            flush_binary_groups(&mut leftover, &mut writer)?;
        }
        if !leftover.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "bit string length is not a multiple of 8",
            ));
        }
        Ok(())
    }

    fn flush_binary_groups(leftover: &mut Vec<u8>, writer: &mut impl Write) -> io::Result<()> {
        let complete_len = (leftover.len() / 8) * 8;
        if complete_len == 0 {
            return Ok(());
        }
        let mut out = Vec::with_capacity(complete_len / 8);
        // bitwise decode instead of UTF-8 string + from_str_radix per byte.
        // avoids allocating a &str for every byte
        for chunk in leftover[..complete_len].chunks(8) {
            let mut value = 0u8;
            for &bit in chunk {
                value = (value << 1) | (bit - b'0');
            }
            out.push(value);
        }
        writer.write_all(&out)?;
        leftover.drain(..complete_len);
        Ok(())
    }

    // hex (1 byte -> 2 chars, no grouping ambiguity)

    pub fn hex_encode(mut reader: impl Read, mut writer: impl Write) -> io::Result<()> {
        let mut buf = [0u8; CHUNK_SIZE];
        loop {
            let n = reader.read(&mut buf)?;
            if n == 0 {
                break;
            }
            let encoded = faster_hex::hex_string(&buf[..n]);
            writer.write_all(encoded.as_bytes())?;
        }
        writer.write_all(b"\n")?;
        Ok(())
    }

    pub fn hex_decode(mut reader: impl Read, mut writer: impl Write) -> io::Result<()> {
        let mut leftover: Vec<u8> = Vec::new();
        let mut buf = [0u8; CHUNK_SIZE];
        loop {
            let n = reader.read(&mut buf)?;
            if n == 0 {
                break;
            }
            leftover.extend(
                buf[..n]
                    .iter()
                    .copied()
                    .filter(|b| !b.is_ascii_whitespace()),
            );
            let complete_len = (leftover.len() / 2) * 2;
            if complete_len > 0 {
                let mut out = vec![0u8; complete_len / 2];
                faster_hex::hex_decode(&leftover[..complete_len], &mut out)
                    .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("{e:?}")))?;
                writer.write_all(&out)?;
                leftover.drain(..complete_len);
            }
        }
        if !leftover.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "odd-length hex input",
            ));
        }
        Ok(())
    }

    // base64 (groups of 3 bytes -> 4 chars)

    pub fn base64_encode(mut reader: impl Read, mut writer: impl Write) -> io::Result<()> {
        let mut leftover: Vec<u8> = Vec::new();
        let mut buf = [0u8; CHUNK_SIZE];
        loop {
            let n = reader.read(&mut buf)?;
            if n == 0 {
                break;
            }
            leftover.extend_from_slice(&buf[..n]);
            let complete_len = (leftover.len() / 3) * 3;
            if complete_len > 0 {
                let encoded = STANDARD.encode(&leftover[..complete_len]);
                writer.write_all(encoded.as_bytes())?;
                leftover.drain(..complete_len);
            }
        }
        if !leftover.is_empty() {
            let encoded = STANDARD.encode(&leftover);
            writer.write_all(encoded.as_bytes())?;
        }
        writer.write_all(b"\n")?;
        Ok(())
    }

    pub fn base64_decode(mut reader: impl Read, mut writer: impl Write) -> io::Result<()> {
        let mut leftover: Vec<u8> = Vec::new();
        let mut buf = [0u8; CHUNK_SIZE];
        loop {
            let n = reader.read(&mut buf)?;
            if n == 0 {
                break;
            }
            leftover.extend(
                buf[..n]
                    .iter()
                    .copied()
                    .filter(|b| !b.is_ascii_whitespace()),
            );
            flush_base64_groups(&mut leftover, &mut writer)?;
        }
        if !leftover.is_empty() {
            let decoded = STANDARD
                .decode(&leftover)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
            writer.write_all(&decoded)?;
        }
        Ok(())
    }

    fn flush_base64_groups(leftover: &mut Vec<u8>, writer: &mut impl Write) -> io::Result<()> {
        let mut complete_len = (leftover.len() / 4) * 4;
        while complete_len > 0 && leftover[..complete_len].contains(&b'=') {
            complete_len -= 4;
        }
        if complete_len > 0 {
            let decoded = STANDARD
                .decode(&leftover[..complete_len])
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
            writer.write_all(&decoded)?;
            leftover.drain(..complete_len);
        }
        Ok(())
    }

    // base32 (groups of 5 bytes -> 8 chars)

    pub fn base32_encode(mut reader: impl Read, mut writer: impl Write) -> io::Result<()> {
        let mut leftover: Vec<u8> = Vec::new();
        let mut buf = [0u8; CHUNK_SIZE];
        loop {
            let n = reader.read(&mut buf)?;
            if n == 0 {
                break;
            }
            leftover.extend_from_slice(&buf[..n]);
            let complete_len = (leftover.len() / 5) * 5;
            if complete_len > 0 {
                let encoded = BASE32.encode(&leftover[..complete_len]);
                writer.write_all(encoded.as_bytes())?;
                leftover.drain(..complete_len);
            }
        }
        if !leftover.is_empty() {
            let encoded = BASE32.encode(&leftover);
            writer.write_all(encoded.as_bytes())?;
        }
        writer.write_all(b"\n")?;
        Ok(())
    }

    pub fn base32_decode(mut reader: impl Read, mut writer: impl Write) -> io::Result<()> {
        let mut leftover: Vec<u8> = Vec::new();
        let mut buf = [0u8; CHUNK_SIZE];
        loop {
            let n = reader.read(&mut buf)?;
            if n == 0 {
                break;
            }
            leftover.extend(
                buf[..n]
                    .iter()
                    .copied()
                    .filter(|b| !b.is_ascii_whitespace()),
            );
            let mut complete_len = (leftover.len() / 8) * 8;
            while complete_len > 0 && leftover[..complete_len].contains(&b'=') {
                complete_len -= 8;
            }
            if complete_len > 0 {
                let decoded = BASE32
                    .decode(&leftover[..complete_len])
                    .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
                writer.write_all(&decoded)?;
                leftover.drain(..complete_len);
            }
        }
        if !leftover.is_empty() {
            let decoded = BASE32
                .decode(&leftover)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
            writer.write_all(&decoded)?;
        }
        Ok(())
    }

    //       base85 (in-memory only; the base85 crate does not expose a
    //       streaming API and RFC 1924 / ASCII85 chunk boundaries depend on
    //       whole-input alignment, so buffering is unavoidable here)

    pub fn base85_run(mut reader: impl Read, decode: bool) -> io::Result<()> {
        let mut input = Vec::new();
        reader.read_to_end(&mut input)?;
        if decode {
            let text = std::str::from_utf8(&input)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?
                .trim_end();
            let decoded = base85::decode(text)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("{e:?}")))?;
            io::stdout().write_all(&decoded)
        } else {
            let encoded = base85::encode(&input);
            println!("{encoded}");
            Ok(())
        }
    }
}

// CLI surface (fallback when invoked by real binary name)

#[derive(Parser)]
#[command(
    name = "xtool",
    about = "Multi-codec CLI: base32/base64/base85/hex/binary/sha256sum/sha512sum"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Base32 {
        #[arg(short = 'd', long)]
        decode: bool,
        file: Option<PathBuf>,
    },
    Base64 {
        #[arg(short = 'd', long)]
        decode: bool,
        file: Option<PathBuf>,
    },
    Base85 {
        #[arg(short = 'd', long)]
        decode: bool,
        file: Option<PathBuf>,
    },
    Hex {
        #[arg(short = 'd', long)]
        decode: bool,
        file: Option<PathBuf>,
    },
    Binary {
        #[arg(short = 'd', long)]
        decode: bool,
        file: Option<PathBuf>,
    },
    Sha256Sum {
        /// One or more files to hash (GNU coreutils-compatible multi-file support).
        files: Vec<PathBuf>,
    },
    Sha512Sum {
        /// One or more files to hash (GNU coreutils-compatible multi-file support).
        files: Vec<PathBuf>,
    },
}

fn parse_encode_decode_args(args: &[String]) -> io::Result<(bool, Option<PathBuf>)> {
    let mut decode = false;
    let mut file: Option<PathBuf> = None;
    for arg in args {
        if arg == "-d" || arg == "--decode" {
            decode = true;
        } else if arg == "-" {
            // FIX: reject a second file argument instead of silently dropping
            // earlier ones.
            if file.is_some() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "multiple input files specified",
                ));
            }
            file = Some(PathBuf::from(arg));
        } else if arg.starts_with('-') {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("unrecognized option '{arg}'"),
            ));
        } else {
            // FIX: same guard for regular filenames.
            if file.is_some() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "multiple input files specified",
                ));
            }
            file = Some(PathBuf::from(arg));
        }
    }
    Ok((decode, file))
}

fn parse_sum_args(args: &[String]) -> io::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for arg in args {
        if arg.starts_with('-') && arg != "-" {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("unrecognized option '{arg}'"),
            ));
        }
        files.push(PathBuf::from(arg));
    }
    Ok(files)
}

fn print_codec_help(prog: &str) -> ! {
    println!("Usage: {prog} [-d|--decode] [FILE]");
    println!();
    println!("Encode or decode data via {prog}.");
    println!("Reads FILE if given ('-' or omitted means stdin).");
    println!();
    println!("Options:");
    println!("  -d, --decode   decode instead of encode");
    println!("  -h, --help     print this help message");
    std::process::exit(0);
}

fn print_sum_help(prog: &str) -> ! {
    println!("Usage: {prog} [FILE]...");
    println!();
    println!("Print {prog} checksums for each FILE (or stdin if none given).");
    println!();
    println!("Options:");
    println!("  -h, --help     print this help message");
    std::process::exit(0);
}

fn die(err: io::Error) -> ! {
    eprintln!("error: {err}");
    std::process::exit(1);
}

// helpers for multi file hashing

/// hash every path in files fall back to stdin when the list is empty.
fn run_sha256_files(files: &[PathBuf]) -> io::Result<()> {
    if files.is_empty() {
        return modes::sha256_run(io::stdin(), None);
    }
    for path in files {
        let reader = open_input(Some(path))?;
        let name = path.to_string_lossy();
        modes::sha256_run(reader, Some(&name))?;
    }
    Ok(())
}

fn run_sha512_files(files: &[PathBuf]) -> io::Result<()> {
    if files.is_empty() {
        return modes::sha512_run(io::stdin(), None);
    }
    for path in files {
        let reader = open_input(Some(path))?;
        let name = path.to_string_lossy();
        modes::sha512_run(reader, Some(&name))?;
    }
    Ok(())
}

// argv[0] dispatch

fn main() {
    if let Err(e) = run() {
        die(e);
    }
}

fn run() -> io::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let prog_name = Path::new(&args[0])
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let rest = &args[1..];

    match prog_name.as_str() {
        "base32" => {
            if rest.iter().any(|a| a == "-h" || a == "--help") {
                print_codec_help("base32");
            }
            let (decode, file) = parse_encode_decode_args(rest)?;
            let reader = open_input(file.as_ref())?;
            let mut writer = stdout_writer();
            let result = if decode {
                modes::base32_decode(reader, &mut writer)
            } else {
                modes::base32_encode(reader, &mut writer)
            };
            writer.flush()?;
            result
        }
        "base64" => {
            if rest.iter().any(|a| a == "-h" || a == "--help") {
                print_codec_help("base64");
            }
            let (decode, file) = parse_encode_decode_args(rest)?;
            let reader = open_input(file.as_ref())?;
            let mut writer = stdout_writer();
            let result = if decode {
                modes::base64_decode(reader, &mut writer)
            } else {
                modes::base64_encode(reader, &mut writer)
            };
            writer.flush()?;
            result
        }
        "base85" => {
            if rest.iter().any(|a| a == "-h" || a == "--help") {
                print_codec_help("base85");
            }
            let (decode, file) = parse_encode_decode_args(rest)?;
            let reader = open_input(file.as_ref())?;
            modes::base85_run(reader, decode)
        }
        "hex" => {
            if rest.iter().any(|a| a == "-h" || a == "--help") {
                print_codec_help("hex");
            }
            let (decode, file) = parse_encode_decode_args(rest)?;
            let reader = open_input(file.as_ref())?;
            let mut writer = stdout_writer();
            let result = if decode {
                modes::hex_decode(reader, &mut writer)
            } else {
                modes::hex_encode(reader, &mut writer)
            };
            writer.flush()?;
            result
        }
        "binary" => {
            if rest.iter().any(|a| a == "-h" || a == "--help") {
                print_codec_help("binary");
            }
            let (decode, file) = parse_encode_decode_args(rest)?;
            let reader = open_input(file.as_ref())?;
            let mut writer = stdout_writer();
            let result = if decode {
                modes::binary_decode(reader, &mut writer)
            } else {
                modes::binary_encode(reader, &mut writer)
            };
            writer.flush()?;
            result
        }
        "sha256sum" => {
            if rest.iter().any(|a| a == "-h" || a == "--help") {
                print_sum_help("sha256sum");
            }
            let files = parse_sum_args(rest)?;
            run_sha256_files(&files)
        }
        "sha512sum" => {
            if rest.iter().any(|a| a == "-h" || a == "--help") {
                print_sum_help("sha512sum");
            }
            let files = parse_sum_args(rest)?;
            run_sha512_files(&files)
        }
        _ => {
            let cli = Cli::parse();
            match cli.command {
                Commands::Base32 { decode, file } => {
                    let reader = open_input(file.as_ref())?;
                    let mut writer = stdout_writer();
                    let result = if decode {
                        modes::base32_decode(reader, &mut writer)
                    } else {
                        modes::base32_encode(reader, &mut writer)
                    };
                    writer.flush()?;
                    result
                }
                Commands::Base64 { decode, file } => {
                    let reader = open_input(file.as_ref())?;
                    let mut writer = stdout_writer();
                    let result = if decode {
                        modes::base64_decode(reader, &mut writer)
                    } else {
                        modes::base64_encode(reader, &mut writer)
                    };
                    writer.flush()?;
                    result
                }
                Commands::Base85 { decode, file } => {
                    let reader = open_input(file.as_ref())?;
                    modes::base85_run(reader, decode)
                }
                Commands::Hex { decode, file } => {
                    let reader = open_input(file.as_ref())?;
                    let mut writer = stdout_writer();
                    let result = if decode {
                        modes::hex_decode(reader, &mut writer)
                    } else {
                        modes::hex_encode(reader, &mut writer)
                    };
                    writer.flush()?;
                    result
                }
                Commands::Binary { decode, file } => {
                    let reader = open_input(file.as_ref())?;
                    let mut writer = stdout_writer();
                    let result = if decode {
                        modes::binary_decode(reader, &mut writer)
                    } else {
                        modes::binary_encode(reader, &mut writer)
                    };
                    writer.flush()?;
                    result
                }
                Commands::Sha256Sum { files } => run_sha256_files(&files),
                Commands::Sha512Sum { files } => run_sha512_files(&files),
            }
        }
    }
}
