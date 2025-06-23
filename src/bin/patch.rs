use clap::Parser;
use diffy::{apply, patch_from_str};
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process;

#[derive(Parser, Debug)]
#[command(name = "diffy-patch")]
#[command(version = env!("CARGO_PKG_VERSION"))]
#[command(about = "Apply a diff file to an original", long_about = None)]
#[command(after_help = "EXAMPLES:
    # Apply a patch to a file
    diffy-patch -i file.txt -p patch.diff
    
    # Apply a patch and save to a different file
    diffy-patch -i original.txt -p changes.patch -o updated.txt
    
    # Apply patch from stdin
    cat patch.diff | diffy-patch -i file.txt
    
    # Create backup before patching
    diffy-patch -i file.txt -p patch.diff --backup
    
    # Apply multi-file patch to directory
    diffy-patch -p multi.patch -d /path/to/project/
    
    # Apply patch with path stripping
    diffy-patch -p patch.diff -p1 -d ./
    
    # Dry run to see what would change
    diffy-patch -i file.txt -p patch.diff --dry-run")]
struct Args {
    /// File to patch (use - for stdin)
    #[arg(short, long, value_name = "FILE")]
    input: Option<PathBuf>,

    /// Output file (default: overwrite input file)
    #[arg(short, long, value_name = "FILE")]
    output: Option<PathBuf>,

    /// Patch file to apply (use - for stdin)
    #[arg(long, value_name = "PATCHFILE")]
    patch: Option<PathBuf>,

    /// Interpret the patch as a reverse patch
    #[arg(short = 'R', long)]
    reverse: bool,

    /// Remove output files instead of creating them
    #[arg(short = 'E', long)]
    remove_empty_files: bool,

    /// Do not actually change any files; just print what would happen
    #[arg(long)]
    dry_run: bool,

    /// Strip NUM leading components from file names
    #[arg(short = 'p', long, default_value = "0")]
    strip: usize,

    /// Base directory for applying patches (when patch contains multiple files)
    #[arg(short = 'd', long, value_name = "DIR")]
    directory: Option<PathBuf>,

    /// Treat input as binary
    #[arg(long)]
    binary: bool,

    /// Be verbose
    #[arg(short, long)]
    verbose: bool,

    /// Create backups of original files
    #[arg(short, long)]
    backup: bool,

    /// Positional patch file argument
    patchfile: Option<PathBuf>,
}

fn read_file_or_stdin(path: Option<&PathBuf>) -> io::Result<Vec<u8>> {
    match path {
        Some(p) if p.to_str() != Some("-") => fs::read(p),
        _ => {
            let mut buffer = Vec::new();
            io::stdin().read_to_end(&mut buffer)?;
            Ok(buffer)
        }
    }
}

fn strip_path_components(path: &str, strip: usize) -> Option<String> {
    let components: Vec<&str> = path.split('/').collect();
    if components.len() > strip {
        Some(components[strip..].join("/"))
    } else {
        None
    }
}

fn extract_filename_from_patch(patch_content: &str, strip: usize) -> Option<String> {
    for line in patch_content.lines() {
        if line.starts_with("--- ") {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 && parts[1] != "/dev/null" {
                let path = parts[1].trim_start_matches("a/").trim_start_matches("b/");
                return strip_path_components(path, strip);
            }
        } else if line.starts_with("+++ ") {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 && parts[1] != "/dev/null" {
                let path = parts[1].trim_start_matches("a/").trim_start_matches("b/");
                return strip_path_components(path, strip);
            }
        }
    }
    None
}

fn split_multi_file_patch(patch_content: &str) -> Vec<(String, String)> {
    let mut patches = Vec::new();
    let mut current_filename = String::new();
    let mut current_patch = String::new();
    let mut in_patch = false;
    
    for line in patch_content.lines() {
        if line.starts_with("diff ") {
            // Start of a new file diff
            if in_patch && !current_patch.is_empty() {
                patches.push((current_filename.clone(), current_patch.clone()));
            }
            current_filename.clear();
            current_patch.clear();
            in_patch = true;
            current_patch.push_str(line);
            current_patch.push('\n');
        } else if line.starts_with("--- ") {
            // Extract filename from --- line
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 && parts[1] != "/dev/null" {
                current_filename = parts[1].trim_start_matches("a/").trim_start_matches("b/").to_string();
            }
            current_patch.push_str(line);
            current_patch.push('\n');
        } else if in_patch {
            current_patch.push_str(line);
            current_patch.push('\n');
        }
    }
    
    // Add the last patch
    if in_patch && !current_patch.is_empty() {
        patches.push((current_filename, current_patch));
    }
    
    patches
}

fn apply_single_patch(
    filename: &str,
    patch_content: &str,
    args: &Args,
) -> Result<(), Box<dyn std::error::Error>> {
    // Parse the patch
    let patch = patch_from_str(patch_content)?;
    
    // Determine the file path
    let base_dir = args.directory.as_ref().map(|d| d.as_path()).unwrap_or(Path::new("."));
    let file_path = if let Some(stripped) = strip_path_components(filename, args.strip) {
        base_dir.join(stripped)
    } else {
        return Err(format!("Cannot strip {} components from {}", args.strip, filename).into());
    };
    
    if args.verbose {
        eprintln!("patching file {}", file_path.display());
    }
    
    // Read input file (or create empty content for new files)
    let input_content = if file_path.exists() {
        fs::read(&file_path)?
    } else {
        if args.verbose {
            eprintln!("creating new file {}", file_path.display());
        }
        // Create parent directories if they don't exist
        if let Some(parent) = file_path.parent() {
            fs::create_dir_all(parent)?;
        }
        Vec::new()
    };
    
    // Apply patch
    let output_content = if args.binary {
        return Err("Binary patch application not supported with text patches".into());
    } else {
        let mut current_str = match String::from_utf8(input_content) {
            Ok(s) => s,
            Err(_) => {
                return Err(format!("Input file {} is not valid UTF-8", file_path.display()).into());
            }
        };
        
        for diff in &patch {
            match apply(&current_str, diff) {
                Ok(s) => current_str = s,
                Err(e) => {
                    return Err(format!("Patch failed for {}: {}", file_path.display(), e).into());
                }
            }
        }
        
        current_str.into_bytes()
    };
    
    // Handle dry run
    if args.dry_run {
        if args.verbose {
            eprintln!("would patch {}", file_path.display());
        }
        return Ok(());
    }
    
    // Create backup if requested
    if args.backup && file_path.exists() {
        let backup_path = format!("{}.orig", file_path.display());
        fs::copy(&file_path, &backup_path)?;
        if args.verbose {
            eprintln!("created backup {}", backup_path);
        }
    }
    
    // Write output
    if args.remove_empty_files && output_content.is_empty() {
        if file_path.exists() {
            fs::remove_file(&file_path)?;
            if args.verbose {
                eprintln!("removed empty file {}", file_path.display());
            }
        }
    } else {
        fs::write(&file_path, &output_content)?;
        if args.verbose {
            eprintln!("patched file {}", file_path.display());
        }
    }
    
    Ok(())
}

fn main() {
    let args = Args::parse();

    // Determine patch source
    let patch_path = args.patch.as_ref().or(args.patchfile.as_ref());
    
    // Read patch
    let patch_content = match read_file_or_stdin(patch_path) {
        Ok(content) => content,
        Err(e) => {
            eprintln!("diffy-patch: error reading patch: {}", e);
            process::exit(1);
        }
    };

    let patch_str = match String::from_utf8(patch_content.clone()) {
        Ok(s) => s,
        Err(_) => {
            eprintln!("diffy-patch: patch file is not valid UTF-8");
            process::exit(1);
        }
    };

    // Check if this looks like a multi-file patch
    let file_patches = split_multi_file_patch(&patch_str);
    
    if file_patches.len() > 1 || (file_patches.len() == 1 && args.input.is_none()) {
        // Multi-file patch or single file patch without explicit input
        if args.verbose {
            eprintln!("applying multi-file patch with {} files", file_patches.len());
        }
        
        let mut had_error = false;
        for (filename, patch_content) in file_patches {
            if let Err(e) = apply_single_patch(&filename, &patch_content, &args) {
                eprintln!("diffy-patch: {}", e);
                had_error = true;
                if !args.verbose {
                    break; // Stop on first error unless verbose
                }
            }
        }
        
        if had_error {
            process::exit(1);
        }
        
        process::exit(0);
    }
    
    // Single file patch with explicit input file
    let patch = match patch_from_str(&patch_str) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("diffy-patch: error parsing patch: {}", e);
            process::exit(1);
        }
    };

    // Reverse patch if requested
    if args.reverse {
        eprintln!("diffy-patch: --reverse not fully implemented yet");
        process::exit(1);
    }

    // Determine input file
    let input_path = match &args.input {
        Some(p) => p.clone(),
        None => {
            eprintln!("diffy-patch: no input file specified");
            process::exit(1);
        }
    };

    if args.verbose {
        eprintln!("patching file {}", input_path.display());
    }

    // Read input file
    let input_content = if input_path.to_str() == Some("-") {
        match read_file_or_stdin(None) {
            Ok(content) => content,
            Err(e) => {
                eprintln!("diffy-patch: error reading stdin: {}", e);
                process::exit(1);
            }
        }
    } else if input_path.exists() {
        match fs::read(&input_path) {
            Ok(content) => content,
            Err(e) => {
                eprintln!("diffy-patch: {}: {}", input_path.display(), e);
                process::exit(1);
            }
        }
    } else {
        // File doesn't exist - this might be a new file creation
        Vec::new()
    };

    // Apply patch - handle multiple diffs in the patch
    let output_content = if args.binary {
        eprintln!("diffy-patch: binary patch application not supported with text patches");
        process::exit(1);
    } else {
        let mut current_str = match String::from_utf8(input_content) {
            Ok(s) => s,
            Err(_) => {
                eprintln!("diffy-patch: input file is not valid UTF-8");
                process::exit(1);
            }
        };
        
        for diff in &patch {
            match apply(&current_str, diff) {
                Ok(s) => current_str = s,
                Err(e) => {
                    eprintln!("diffy-patch: patch failed: {}", e);
                    process::exit(1);
                }
            }
        }
        
        current_str.into_bytes()
    };

    // Handle dry run
    if args.dry_run {
        if args.verbose {
            eprintln!("diffy-patch: dry run - no changes made");
        }
        process::exit(0);
    }

    // Create backup if requested
    if args.backup && input_path.to_str() != Some("-") && input_path.exists() {
        let backup_path = format!("{}.orig", input_path.display());
        if let Err(e) = fs::copy(&input_path, &backup_path) {
            eprintln!("diffy-patch: error creating backup: {}", e);
            process::exit(1);
        }
        if args.verbose {
            eprintln!("created backup {}", backup_path);
        }
    }

    // Write output
    let output_path = args.output.as_ref().unwrap_or(&input_path);
    
    if output_path.to_str() == Some("-") {
        // Write to stdout
        if let Err(e) = io::stdout().write_all(&output_content) {
            eprintln!("diffy-patch: error writing to stdout: {}", e);
            process::exit(1);
        }
    } else {
        // Write to file
        if args.remove_empty_files && output_content.is_empty() {
            if output_path.exists() {
                if let Err(e) = fs::remove_file(output_path) {
                    eprintln!("diffy-patch: error removing empty file: {}", e);
                    process::exit(1);
                }
                if args.verbose {
                    eprintln!("removed empty file {}", output_path.display());
                }
            }
        } else {
            if let Err(e) = fs::write(output_path, &output_content) {
                eprintln!("diffy-patch: error writing output: {}", e);
                process::exit(1);
            }
            if args.verbose {
                eprintln!("patched file {}", output_path.display());
            }
        }
    }

    process::exit(0);
}