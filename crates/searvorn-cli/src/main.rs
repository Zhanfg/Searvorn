use std::{
    env,
    io::{self, Write},
    path::PathBuf,
    process::ExitCode,
};

use searvorn_core::{
    apk::inspect_apk,
    apk_signing::{signing_certificates, ApkSigningScheme},
    hex::{read_hex_rows_into, HexLayout},
    sha256::format_hex,
    text::{probe_text_default, TextEncoding},
    vfs::{NodeType, VfsBackend},
    zip::scan_zip,
    LocalFsBackend, SearvornError,
};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("searvorn: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args().skip(1);
    let mut root = PathBuf::from(".");

    let first = args.next();
    let command = match first.as_deref() {
        Some("--root") => {
            root = PathBuf::from(args.next().ok_or("missing value for --root")?);
            args.next().ok_or("missing command")?
        }
        Some(command) => command.to_owned(),
        None => return Err("missing command".into()),
    };

    let backend = LocalFsBackend::new(root, false)?;

    match command.as_str() {
        "ls" => {
            let path = args.next().unwrap_or_else(|| "/".to_owned());
            list(&backend, &path)?;
            reject_extra(args)?;
        }
        "stat" => {
            let path = args.next().ok_or("stat requires a path")?;
            stat(&backend, &path)?;
            reject_extra(args)?;
        }
        "cat" => {
            let path = args.next().ok_or("cat requires a path")?;
            cat(&backend, &path)?;
            reject_extra(args)?;
        }
        "zip-list" => {
            let path = args.next().ok_or("zip-list requires a path")?;
            zip_list(&backend, &path)?;
            reject_extra(args)?;
        }
        "apk-info" => {
            let path = args.next().ok_or("apk-info requires a path")?;
            apk_info(&backend, &path)?;
            reject_extra(args)?;
        }
        "text-probe" => {
            let path = args.next().ok_or("text-probe requires a path")?;
            text_probe(&backend, &path)?;
            reject_extra(args)?;
        }
        "hex" => {
            let path = args.next().ok_or("hex requires a path")?;
            let offset = args
                .next()
                .map(|value| value.parse::<u64>())
                .transpose()?
                .unwrap_or(0);
            let length = args
                .next()
                .map(|value| value.parse::<usize>())
                .transpose()?
                .unwrap_or(256);
            reject_extra(args)?;
            hex_dump(&backend, &path, offset, length)?;
        }
        _ => return Err(format!("unknown command: {command}").into()),
    }

    Ok(())
}

fn reject_extra(mut args: impl Iterator<Item = String>) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(extra) = args.next() {
        Err(format!("unexpected argument: {extra}").into())
    } else {
        Ok(())
    }
}

fn list(backend: &LocalFsBackend, path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let mut output = io::BufWriter::new(io::stdout().lock());

    backend.read_dir(path, &mut |entry| {
        writeln!(
            output,
            "{}\t{}\t{}",
            type_name(entry.metadata.node_type),
            entry.metadata.len,
            entry.name
        )
        .map_err(|error| SearvornError::from_io("cli.ls", error))
    })?;

    output.flush()?;
    Ok(())
}

fn stat(backend: &LocalFsBackend, path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let metadata = backend.metadata(path)?;

    println!("type={}", type_name(metadata.node_type));
    println!("size={}", metadata.len);

    Ok(())
}

fn cat(backend: &LocalFsBackend, path: &str) -> Result<(), Box<dyn std::error::Error>> {
    const CHUNK: usize = 64 * 1024;

    let mut reader = backend.open_read(path)?;
    let mut stdout = io::stdout().lock();
    let mut buffer = vec![0u8; CHUNK];
    let mut offset = 0u64;

    loop {
        let read = reader.read_at(offset, &mut buffer)?;
        if read == 0 {
            break;
        }

        stdout.write_all(&buffer[..read])?;
        offset += read as u64;
    }

    stdout.flush()?;
    Ok(())
}

fn zip_list(backend: &LocalFsBackend, path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let mut reader = backend.open_read(path)?;
    let mut output = io::BufWriter::new(io::stdout().lock());

    let summary = scan_zip(reader.as_mut(), |entry| {
        writeln!(
            output,
            "{}\t{}\t{}\t{}",
            entry.compression_method,
            entry.compressed_size,
            entry.uncompressed_size,
            entry.display_name()
        )
        .map_err(|error| SearvornError::from_io("cli.zip_list", error))
    })?;

    output.flush()?;
    eprintln!(
        "entries={} central_offset={} central_size={}",
        summary.entries, summary.central_directory_offset, summary.central_directory_size
    );
    Ok(())
}

fn apk_info(backend: &LocalFsBackend, path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let mut reader = backend.open_read(path)?;
    let summary = inspect_apk(reader.as_mut())?;

    println!("entries={}", summary.total_entries);
    println!("manifest={}", summary.has_android_manifest);
    println!("resources_arsc={}", summary.has_resources_arsc);
    println!("dex_files={}", summary.dex_files);
    println!("native_libraries={}", summary.native_libraries);
    println!("native_abis={}", summary.native_abis.join(","));
    println!("assets={}", summary.asset_entries);
    println!("resources={}", summary.resource_entries);
    println!("meta_inf={}", summary.meta_inf_entries);
    println!("v1_signature_files={}", summary.v1_signature_files);
    println!("signing_v2={}", summary.has_v2_signing);
    println!("signing_v3={}", summary.has_v3_signing);
    println!("signing_v31={}", summary.has_v31_signing);

    let zip = searvorn_core::zip::scan_zip(reader.as_mut(), |_| Ok(()))?;
    let certificates = signing_certificates(reader.as_mut(), zip.central_directory_offset)?;
    for certificate in certificates {
        println!(
            "certificate={} signer={} chain={} sha256={}",
            signing_scheme_name(certificate.scheme),
            certificate.signer_index,
            certificate.certificate_index,
            format_hex(&certificate.sha256)
        );
    }

    Ok(())
}

fn text_probe(backend: &LocalFsBackend, path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let mut reader = backend.open_read(path)?;
    let probe = probe_text_default(reader.as_mut())?;

    println!("encoding={}", encoding_name(probe.encoding));
    println!("sampled_bytes={}", probe.sampled_bytes);
    println!("contains_nul={}", probe.contains_nul);
    println!("likely_binary={}", probe.likely_binary());
    Ok(())
}

fn hex_dump(
    backend: &LocalFsBackend,
    path: &str,
    offset: u64,
    length: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    let layout = HexLayout::default();
    let first_row = layout.row_for_offset(offset);
    let rows = length.div_ceil(layout.bytes_per_row());
    let mut buffer = vec![0u8; rows.saturating_mul(layout.bytes_per_row())];
    let mut reader = backend.open_read(path)?;
    let window = read_hex_rows_into(reader.as_mut(), layout, first_row, rows, &mut buffer)?;

    let bytes = &buffer[..window.bytes_read];
    for (row_index, row) in bytes.chunks(layout.bytes_per_row()).enumerate() {
        let row_offset = window.byte_offset + (row_index * layout.bytes_per_row()) as u64;
        print!("{row_offset:08x}  ");

        for column in 0..layout.bytes_per_row() {
            if let Some(byte) = row.get(column) {
                print!("{byte:02x} ");
            } else {
                print!("   ");
            }
        }

        print!(" |");
        for byte in row {
            let character = if byte.is_ascii_graphic() || *byte == b' ' {
                char::from(*byte)
            } else {
                '.'
            };
            print!("{character}");
        }
        println!("|");
    }

    Ok(())
}

fn encoding_name(encoding: TextEncoding) -> &'static str {
    match encoding {
        TextEncoding::Utf8 => "utf-8",
        TextEncoding::Utf8Bom => "utf-8-bom",
        TextEncoding::Utf16LeBom => "utf-16le-bom",
        TextEncoding::Utf16BeBom => "utf-16be-bom",
        TextEncoding::Unknown => "unknown",
    }
}

fn type_name(node_type: NodeType) -> &'static str {
    match node_type {
        NodeType::File => "file",
        NodeType::Directory => "dir",
        NodeType::Symlink => "link",
        NodeType::Other => "other",
    }
}

fn signing_scheme_name(scheme: ApkSigningScheme) -> &'static str {
    match scheme {
        ApkSigningScheme::V2 => "v2",
        ApkSigningScheme::V3 => "v3",
        ApkSigningScheme::V31 => "v3.1",
    }
}
