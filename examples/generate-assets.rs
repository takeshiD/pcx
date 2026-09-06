use std::{env, fs, io, path::PathBuf};

use clap_complete::{generate_to, shells};

fn generate_manpages(mut command: clap::Command, output: &std::path::Path) -> io::Result<()> {
    command = command.disable_help_subcommand(true);
    command.build();
    for subcommand in command.get_subcommands().cloned() {
        generate_manpages(subcommand, output)?;
    }
    let path = clap_mangen::Man::new(command)
        .source(format!("pcx {}", env!("CARGO_PKG_VERSION")))
        .generate_to(output)?;
    let rendered = fs::read_to_string(&path)?;
    let normalized = rendered
        .lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    fs::write(path, normalized)?;
    Ok(())
}

fn main() -> io::Result<()> {
    let output = env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .ok_or_else(|| io::Error::other("usage: generate-assets OUTPUT_DIRECTORY"))?;
    if env::args_os().nth(2).is_some() {
        return Err(io::Error::other("usage: generate-assets OUTPUT_DIRECTORY"));
    }

    let completions = output.join("completions");
    let man = output.join("man");
    fs::create_dir_all(&completions)?;
    fs::create_dir_all(&man)?;

    generate_to(
        shells::Bash,
        &mut pcx_cli::cli::command(),
        "pcx",
        &completions,
    )?;
    generate_to(
        shells::Zsh,
        &mut pcx_cli::cli::command(),
        "pcx",
        &completions,
    )?;
    generate_to(
        shells::Fish,
        &mut pcx_cli::cli::command(),
        "pcx",
        &completions,
    )?;
    generate_manpages(pcx_cli::cli::command(), &man)
}
