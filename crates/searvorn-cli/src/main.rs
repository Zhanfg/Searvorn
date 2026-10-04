use std::{
    env,
    io::{self, Write},
    path::PathBuf,
    process::ExitCode,
};

use searvorn_core::{
    vfs::{NodeType, VfsBackend},
    LocalFsBackend,
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
        }
        "stat" => {
            let path = args.next().ok_or("stat requires a path")?;
            stat(&backend, &path)?;
        }
        "cat" => {
            let path = args.next().ok_or("cat requires a path")?;
            cat(&backend, &path)?;
        }
        _ => return Err(format!("unknown command: {command}").into()),
    }

    if let Some(extra) = args.next() {
        return Err(format!("unexpected argument: {extra}").into());
    }

    Ok(())
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
        .map_err(|error| searvorn_core::SearvornError::from_io("cli.ls", error))
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

fn type_name(node_type: NodeType) -> &'static str {
    match node_type {
        NodeType::File => "file",
        NodeType::Directory => "dir",
        NodeType::Symlink => "link",
        NodeType::Other => "other",
    }
}
