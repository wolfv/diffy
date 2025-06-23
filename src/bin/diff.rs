use clap::{Parser, ValueEnum};
use diffy::{create_patch_bytes, DiffOptions, PatchFormatter};
use std::collections::BTreeSet;
use std::fs;
use std::io::{self, Read, IsTerminal};
use std::path::{Path, PathBuf};
use std::process;

#[derive(Debug, Clone, ValueEnum)]
enum ColorChoice {
    Always,
    Never,
    Auto,
}

#[derive(Parser, Debug)]
#[command(name = "diffy-diff")]
#[command(version = env!("CARGO_PKG_VERSION"))]
#[command(about = "Compare files line by line", long_about = None)]
#[command(after_help = "EXAMPLES:
    # Compare two files
    diffy-diff file1.txt file2.txt
    
    # Compare with colored output
    diffy-diff --color=always file1.txt file2.txt
    
    # Compare with more context lines
    diffy-diff -C 5 file1.txt file2.txt
    
    # Compare ignoring case
    diffy-diff -i file1.txt file2.txt
    
    # Compare directories
    diffy-diff dir1/ dir2/
    
    # Compare directories recursively
    diffy-diff -r dir1/ dir2/
    
    # Compare directories with exclusions
    diffy-diff -r --exclude=\"*.log\" dir1/ dir2/
    
    # Read from stdin
    cat file1.txt | diffy-diff - file2.txt")]
struct Args {
    /// First file to compare (use - for stdin)
    file1: PathBuf,

    /// Second file to compare
    file2: PathBuf,

    /// When to use colors: always, never, or auto (default: auto)
    #[arg(short, long, value_enum, default_value = "auto")]
    color: ColorChoice,

    /// Ignore case differences
    #[arg(short = 'i', long)]
    ignore_case: bool,

    /// Ignore all white space
    #[arg(short = 'w', long)]
    ignore_all_space: bool,

    /// Ignore changes in the amount of white space
    #[arg(short = 'b', long)]
    ignore_space_change: bool,

    /// Use NUM lines of context
    #[arg(short = 'C', long = "context", default_value = "3")]
    context: usize,

    /// Output in unified format
    #[arg(short = 'u', long)]
    unified: bool,

    /// Treat files as binary
    #[arg(long)]
    binary: bool,

    /// Compare directories recursively
    #[arg(short = 'r', long)]
    recursive: bool,

    /// Ignore files matching these patterns (glob syntax)
    #[arg(long, value_name = "PATTERN")]
    exclude: Vec<String>,
}

fn read_file_or_stdin(path: &PathBuf) -> io::Result<Vec<u8>> {
    if path.to_str() == Some("-") {
        let mut buffer = Vec::new();
        io::stdin().read_to_end(&mut buffer)?;
        Ok(buffer)
    } else {
        fs::read(path)
    }
}

fn collect_files(dir: &Path, recursive: bool, exclude_patterns: &[String]) -> io::Result<BTreeSet<PathBuf>> {
    let mut files = BTreeSet::new();
    
    if !dir.is_dir() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "Not a directory"));
    }
    
    fn visit_dir(
        dir: &Path, 
        files: &mut BTreeSet<PathBuf>, 
        recursive: bool, 
        exclude_patterns: &[String],
        base_dir: &Path
    ) -> io::Result<()> {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            let relative_path = path.strip_prefix(base_dir).unwrap_or(&path);
            
            // Check if path matches any exclude pattern
            let path_str = relative_path.to_string_lossy();
            let should_exclude = exclude_patterns.iter().any(|pattern| {
                // Simple glob matching - could be enhanced with a proper glob library
                if pattern.contains('*') {
                    let pattern_parts: Vec<&str> = pattern.split('*').collect();
                    if pattern_parts.len() == 2 {
                        path_str.starts_with(pattern_parts[0]) && path_str.ends_with(pattern_parts[1])
                    } else {
                        false
                    }
                } else {
                    path_str == pattern.as_str()
                }
            });
            
            if should_exclude {
                continue;
            }
            
            if path.is_file() {
                files.insert(relative_path.to_path_buf());
            } else if path.is_dir() && recursive {
                visit_dir(&path, files, recursive, exclude_patterns, base_dir)?;
            }
        }
        Ok(())
    }
    
    visit_dir(dir, &mut files, recursive, exclude_patterns, dir)?;
    Ok(files)
}

fn compare_directories(
    dir1: &Path,
    dir2: &Path,
    args: &Args,
) -> Result<bool, Box<dyn std::error::Error>> {
    let files1 = collect_files(dir1, args.recursive, &args.exclude)?;
    let files2 = collect_files(dir2, args.recursive, &args.exclude)?;
    
    let all_files: BTreeSet<_> = files1.union(&files2).cloned().collect();
    let mut has_differences = false;
    
    // Determine whether to use color
    let use_color = match args.color {
        ColorChoice::Always => true,
        ColorChoice::Never => false,
        ColorChoice::Auto => io::stdout().is_terminal(),
    };
    
    for file_path in all_files {
        let path1 = dir1.join(&file_path);
        let path2 = dir2.join(&file_path);
        
        let file1_exists = path1.exists();
        let file2_exists = path2.exists();
        
        if !file1_exists && file2_exists {
            // File only in dir2 (new file)
            has_differences = true;
            println!("Only in {}: {}", dir2.display(), file_path.display());
        } else if file1_exists && !file2_exists {
            // File only in dir1 (deleted file)
            has_differences = true;
            println!("Only in {}: {}", dir1.display(), file_path.display());
        } else if file1_exists && file2_exists {
            // File in both directories - compare content
            let content1 = match fs::read(&path1) {
                Ok(content) => content,
                Err(e) => {
                    eprintln!("diffy-diff: {}: {}", path1.display(), e);
                    continue;
                }
            };
            
            let content2 = match fs::read(&path2) {
                Ok(content) => content,
                Err(e) => {
                    eprintln!("diffy-diff: {}: {}", path2.display(), e);
                    continue;
                }
            };
            
            // Compare files
            if content1 != content2 {
                has_differences = true;
                
                // Print diff header in standard format
                println!("diff --git a/{} b/{}", file_path.display(), file_path.display());
                
                if args.binary || content1.iter().any(|&b| b == 0) || content2.iter().any(|&b| b == 0) {
                    // Binary file
                    println!("Binary files {} and {} differ", path1.display(), path2.display());
                } else {
                    // Text file - show diff
                    let text1 = match String::from_utf8(content1) {
                        Ok(text) => text,
                        Err(_) => {
                            println!("Binary files {} and {} differ", path1.display(), path2.display());
                            continue;
                        }
                    };
                    
                    let text2 = match String::from_utf8(content2) {
                        Ok(text) => text,
                        Err(_) => {
                            println!("Binary files {} and {} differ", path1.display(), path2.display());
                            continue;
                        }
                    };
                    
                    // Apply text transformations
                    let (text1, text2) = if args.ignore_case {
                        (text1.to_lowercase(), text2.to_lowercase())
                    } else {
                        (text1, text2)
                    };
                    
                    let (text1, text2) = if args.ignore_all_space {
                        (
                            text1.chars().filter(|c| !c.is_whitespace()).collect(),
                            text2.chars().filter(|c| !c.is_whitespace()).collect(),
                        )
                    } else if args.ignore_space_change {
                        (
                            text1.split_whitespace().collect::<Vec<_>>().join(" "),
                            text2.split_whitespace().collect::<Vec<_>>().join(" "),
                        )
                    } else {
                        (text1, text2)
                    };
                    
                    // Create diff
                    let mut options = DiffOptions::new();
                    options.set_context_len(args.context);
                    let patch = options.create_patch(&text1, &text2);
                    
                    if !patch.hunks().is_empty() {
                        // Print filenames in proper format
                        println!("--- a/{}", file_path.display());
                        println!("+++ b/{}", file_path.display());
                        
                        // Output diff (but skip the default header since we provide our own)
                        let patch_str = if use_color {
                            let formatter = PatchFormatter::new().with_color();
                            let formatted = formatter.fmt_patch(&patch);
                            formatted.to_string()
                        } else {
                            patch.to_string()
                        };
                        
                        // Skip the first two lines (--- and +++ headers) from the patch
                        let lines: Vec<&str> = patch_str.lines().collect();
                        if lines.len() > 2 {
                            for line in &lines[2..] {
                                println!("{}", line);
                            }
                        }
                    }
                }
            }
        }
    }
    
    Ok(has_differences)
}

fn main() {
    let args = Args::parse();

    // Check if we're comparing directories
    let path1_is_dir = args.file1.is_dir();
    let path2_is_dir = args.file2.is_dir();
    
    if path1_is_dir || path2_is_dir {
        if !path1_is_dir || !path2_is_dir {
            eprintln!("diffy-diff: cannot compare directory and file");
            process::exit(1);
        }
        
        // Compare directories
        match compare_directories(&args.file1, &args.file2, &args) {
            Ok(has_differences) => {
                process::exit(if has_differences { 1 } else { 0 });
            }
            Err(e) => {
                eprintln!("diffy-diff: {}", e);
                process::exit(1);
            }
        }
    }

    // Read files
    let content1 = match read_file_or_stdin(&args.file1) {
        Ok(content) => content,
        Err(e) => {
            eprintln!("diffy-diff: {}: {}", args.file1.display(), e);
            process::exit(1);
        }
    };

    let content2 = match read_file_or_stdin(&args.file2) {
        Ok(content) => content,
        Err(e) => {
            eprintln!("diffy-diff: {}: {}", args.file2.display(), e);
            process::exit(1);
        }
    };

    // Handle binary files
    if args.binary {
        let patch = create_patch_bytes(&content1, &content2);
        
        // For binary patches, check if they differ
        if !patch.hunks().is_empty() {
            println!("Binary files {} and {} differ", 
                args.file1.display(), 
                args.file2.display());
        }
        process::exit(if patch.hunks().is_empty() { 0 } else { 1 });
    }

    // Convert to strings for text comparison
    let text1 = match String::from_utf8(content1) {
        Ok(text) => text,
        Err(_) => {
            eprintln!("diffy-diff: {}: Binary file detected. Use --binary flag.", args.file1.display());
            process::exit(1);
        }
    };

    let text2 = match String::from_utf8(content2) {
        Ok(text) => text,
        Err(_) => {
            eprintln!("diffy-diff: {}: Binary file detected. Use --binary flag.", args.file2.display());
            process::exit(1);
        }
    };

    // Apply text transformations based on flags
    let (text1, text2) = if args.ignore_case {
        (text1.to_lowercase(), text2.to_lowercase())
    } else {
        (text1, text2)
    };

    let (text1, text2) = if args.ignore_all_space {
        (
            text1.chars().filter(|c| !c.is_whitespace()).collect(),
            text2.chars().filter(|c| !c.is_whitespace()).collect(),
        )
    } else if args.ignore_space_change {
        (
            text1.split_whitespace().collect::<Vec<_>>().join(" "),
            text2.split_whitespace().collect::<Vec<_>>().join(" "),
        )
    } else {
        (text1, text2)
    };

    // Create diff
    let mut options = DiffOptions::new();
    options.set_context_len(args.context);
    let patch = options.create_patch(&text1, &text2);
    
    // Check if files are identical
    if patch.hunks().is_empty() {
        // Files are identical, no output
        process::exit(0);
    }

    // Determine whether to use color
    let use_color = match args.color {
        ColorChoice::Always => true,
        ColorChoice::Never => false,
        ColorChoice::Auto => io::stdout().is_terminal(),
    };
    
    // Output diff
    if use_color {
        let formatter = PatchFormatter::new().with_color();
        print!("{}", formatter.fmt_patch(&patch));
    } else {
        print!("{}", patch);
    }

    // Exit with status 1 if files differ
    process::exit(1);
}