use anyhow::Result;
use clap::Parser;
use similarity_core::generic_parser_config::GenericParserConfig;
use similarity_core::generic_tree_sitter_parser::GenericTreeSitterParser;
use similarity_core::language_parser::LanguageParser;
use similarity_core::tsed::{calculate_tsed, TSEDOptions};
use similarity_core::APTEDOptions;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

// Include auto-generated language configs
include!(concat!(env!("OUT_DIR"), "/language_configs.rs"));

#[derive(Parser)]
#[command(name = "similarity-generic")]
#[command(about = "Generic code similarity analyzer using tree-sitter")]
struct Cli {
    /// Path to analyze
    #[arg(required_unless_present_any = ["supported", "show_config"])]
    path: Vec<PathBuf>,

    /// Return 1 for clones and 2 for operational errors (default remains report-only)
    #[arg(long)]
    fail_on_violation: bool,

    /// Language configuration file (JSON)
    #[arg(short, long, conflicts_with_all = ["language", "supported", "show_config"])]
    config: Option<PathBuf>,

    /// Language name (if using built-in config)
    #[arg(short, long, conflicts_with_all = ["config", "supported", "show_config"])]
    language: Option<String>,

    /// Similarity threshold (0.0-1.0)
    #[arg(short, long, default_value = "0.85")]
    threshold: f64,

    /// Show extracted functions
    #[arg(long)]
    show_functions: bool,

    /// Show supported languages
    #[arg(long, conflicts_with_all = ["path", "config", "language", "show_functions", "show_config"])]
    supported: bool,

    /// Show example configuration for a language
    #[arg(long, value_name = "LANGUAGE", conflicts_with_all = ["path", "config", "language", "show_functions", "supported"])]
    show_config: Option<String>,

    /// Enable experimental overlap detection mode
    #[arg(long = "experimental-overlap")]
    overlap: bool,

    /// Minimum window size for overlap detection (number of nodes)
    #[arg(long, default_value = "8")]
    overlap_min_window: u32,

    /// Maximum window size for overlap detection (number of nodes)
    #[arg(long, default_value = "25")]
    overlap_max_window: u32,

    /// Size tolerance for overlap detection (0.0-1.0)
    #[arg(long, default_value = "0.25")]
    overlap_size_tolerance: f64,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let fail = cli.fail_on_violation;
    match run(cli) {
        Ok(matches) if fail && matches > 0 => ExitCode::from(1),
        Ok(_) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error:#}");
            ExitCode::from(if fail { 2 } else { 1 })
        }
    }
}

fn run(cli: Cli) -> Result<usize> {
    if !cli.threshold.is_finite() || !(0.0..=1.0).contains(&cli.threshold) {
        anyhow::bail!("Threshold must be a finite number between 0 and 1");
    }

    // Handle --supported option
    if cli.supported {
        println!("Supported languages for generic tree-sitter parser:");
        println!("  go         - Go language");
        println!("  java       - Java language");
        println!("  c          - C language");
        println!("  cpp        - C++ language");
        println!("  csharp     - C# language");
        println!("  ruby       - Ruby language");
        println!();
        println!("Note: For Python, TypeScript, and Rust, use the dedicated implementations:");
        println!("  similarity-py  - Optimized Python analyzer");
        println!("  similarity-ts  - Optimized TypeScript/JavaScript analyzer");
        println!("  similarity-rs  - (future) Optimized Rust analyzer");
        return Ok(0);
    }

    // Handle --show-config option
    if let Some(lang) = &cli.show_config {
        let config = match lang.as_str() {
            "go" => GenericParserConfig::go(),
            "java" => GenericParserConfig::java(),
            "c" => GenericParserConfig::c(),
            "cpp" | "c++" => GenericParserConfig::cpp(),
            "csharp" | "cs" => GenericParserConfig::csharp(),
            "ruby" | "rb" => GenericParserConfig::ruby(),
            _ => {
                return Err(anyhow::anyhow!(
                    "Unknown language: {}. Use --supported to see available languages.",
                    lang
                ));
            }
        };

        let json = serde_json::to_string_pretty(&config)?;
        println!("{json}");
        return Ok(0);
    }

    // Normal parsing mode
    let path = cli.path.first().ok_or_else(|| anyhow::anyhow!("Path is required"))?.clone();

    let config = if let Some(config_path) = &cli.config {
        GenericParserConfig::from_file(config_path)
            .map_err(|e| anyhow::anyhow!("Failed to load config: {}", e))?
    } else if let Some(lang) = &cli.language {
        // First try to load from embedded configs
        if let Some(config_json) =
            LANGUAGE_CONFIGS.get(lang.as_str()).or_else(|| match lang.as_str() {
                "cpp" => LANGUAGE_CONFIGS.get("cpp"),
                "c++" => LANGUAGE_CONFIGS.get("cpp"),
                "csharp" => LANGUAGE_CONFIGS.get("csharp"),
                "cs" => LANGUAGE_CONFIGS.get("csharp"),
                "ruby" => LANGUAGE_CONFIGS.get("ruby"),
                "rb" => LANGUAGE_CONFIGS.get("ruby"),
                _ => None,
            })
        {
            serde_json::from_str(config_json)
                .map_err(|e| anyhow::anyhow!("Failed to parse embedded config: {}", e))?
        } else {
            // Fall back to hardcoded configs
            match lang.as_str() {
                "go" => GenericParserConfig::go(),
                "java" => GenericParserConfig::java(),
                "c" => GenericParserConfig::c(),
                "cpp" | "c++" => GenericParserConfig::cpp(),
                "csharp" | "cs" => GenericParserConfig::csharp(),
                "ruby" | "rb" => GenericParserConfig::ruby(),
                _ => {
                    eprintln!("Error: Language '{lang}' is not supported by similarity-generic.");
                    eprintln!("Use --supported to see available languages.");
                    if matches!(
                        lang.as_str(),
                        "python" | "py" | "rust" | "rs" | "javascript" | "js" | "typescript" | "ts"
                    ) {
                        eprintln!();
                        eprintln!("Note: For {lang}, use the dedicated implementation:");
                        match lang.as_str() {
                            "python" | "py" => eprintln!("  similarity-py"),
                            "rust" | "rs" => eprintln!("  similarity-rs (planned)"),
                            "javascript" | "js" | "typescript" | "ts" => {
                                eprintln!("  similarity-ts")
                            }
                            _ => {}
                        }
                    }
                    return Err(anyhow::anyhow!("Unsupported language"));
                }
            }
        }
    } else {
        return Err(anyhow::anyhow!("Either --config or --language must be provided"));
    };

    // Create parser based on language
    let language = match config.language.as_str() {
        "go" => tree_sitter_go::LANGUAGE.into(),
        "java" => tree_sitter_java::LANGUAGE.into(),
        "c" => tree_sitter_c::LANGUAGE.into(),
        "cpp" => tree_sitter_cpp::LANGUAGE.into(),
        "csharp" => tree_sitter_c_sharp::LANGUAGE.into(),
        "ruby" => tree_sitter_ruby::LANGUAGE.into(),
        _ => return Err(anyhow::anyhow!("Unsupported language: {}", config.language)),
    };

    let mut parser = GenericTreeSitterParser::new(language, config.clone())
        .map_err(|e| anyhow::anyhow!("Failed to create parser: {}", e))?;

    if config.language == "go" && !cli.overlap {
        return compare_go_files(&mut parser, &cli.path, cli.threshold, cli.show_functions);
    }
    if cli.path.len() != 1 {
        anyhow::bail!("Multiple input files are supported only for Go function comparison");
    }

    // Read file
    let content = fs::read_to_string(&path)?;
    let filename = path.to_string_lossy();

    // Run appropriate analysis based on mode
    if cli.overlap {
        // Overlap detection mode
        check_overlaps(
            path,
            parser,
            cli.threshold,
            cli.overlap_min_window,
            cli.overlap_max_window,
            cli.overlap_size_tolerance,
        )?;
    } else {
        // Normal similarity detection mode
        // Extract functions
        let functions = parser
            .extract_functions(&content, &filename)
            .map_err(|e| anyhow::anyhow!("Failed to extract functions: {}", e))?;

        if cli.show_functions {
            println!("Found {} functions:", functions.len());
            for func in &functions {
                println!("  {} {}:{}-{}", func.name, filename, func.start_line, func.end_line);
            }
            println!();
        }

        // Compare functions
        if functions.len() >= 2 {
            println!("Comparing functions for similarity...");

            let tsed_options = TSEDOptions {
                apted_options: APTEDOptions {
                    rename_cost: 0.3,
                    delete_cost: 1.0,
                    insert_cost: 1.0,
                    compare_values: false,
                },
                min_lines: 1,
                min_tokens: None,
                size_penalty: false,
                skip_test: false,
            };

            for i in 0..functions.len() {
                for j in (i + 1)..functions.len() {
                    let func1 = &functions[i];
                    let func2 = &functions[j];

                    // Extract function bodies
                    let lines: Vec<&str> = content.lines().collect();
                    let body1 =
                        extract_function_body(&lines, func1.body_start_line, func1.body_end_line);
                    let body2 =
                        extract_function_body(&lines, func2.body_start_line, func2.body_end_line);

                    // Parse and compare
                    let tree1 =
                        parser.parse(&body1, &format!("{}:{}", filename, func1.name)).map_err(
                            |e| anyhow::anyhow!("Failed to parse function {}: {}", func1.name, e),
                        )?;
                    let tree2 =
                        parser.parse(&body2, &format!("{}:{}", filename, func2.name)).map_err(
                            |e| anyhow::anyhow!("Failed to parse function {}: {}", func2.name, e),
                        )?;

                    let similarity = calculate_tsed(&tree1, &tree2, &tsed_options);

                    if similarity >= cli.threshold {
                        println!("  {} <-> {}: {:.2}%", func1.name, func2.name, similarity * 100.0);
                    }
                }
            }
        }
    }

    Ok(0)
}

fn compare_go_files(
    parser: &mut GenericTreeSitterParser,
    paths: &[PathBuf],
    threshold: f64,
    show_functions: bool,
) -> Result<usize> {
    let mut paths = paths.iter().map(fs::canonicalize).collect::<std::io::Result<Vec<_>>>()?;
    paths.sort();
    paths.dedup();
    let mut functions = Vec::new();
    for path in paths {
        let source = fs::read_to_string(&path)?;
        let bodies = parser
            .extract_go_function_bodies(&source, &path.to_string_lossy())
            .map_err(|error| anyhow::anyhow!("{error}"))?;
        if show_functions {
            let inventory = parser
                .extract_functions(&source, &path.to_string_lossy())
                .map_err(|error| anyhow::anyhow!("{error}"))?;
            println!("Found {} functions:", inventory.len());
            for function in inventory {
                println!(
                    "  {} {}:{}-{}",
                    function.name,
                    path.display(),
                    function.start_line,
                    function.end_line
                );
            }
            println!();
        }
        functions.extend(bodies.into_iter().map(|body| (path.clone(), body)));
    }
    let options = TSEDOptions {
        apted_options: APTEDOptions {
            rename_cost: 0.3,
            delete_cost: 1.0,
            insert_cost: 1.0,
            compare_values: true,
        },
        min_lines: 1,
        min_tokens: None,
        size_penalty: false,
        skip_test: false,
    };
    let mut matches = 0;
    for (i, (left_path, left)) in functions.iter().enumerate() {
        for (right_path, right) in &functions[i + 1..] {
            let score = calculate_tsed(&left.tree, &right.tree, &options);
            if score >= threshold {
                println!(
                    "  {} <-> {}: {:.2}% ({} <-> {})",
                    left.definition.name,
                    right.definition.name,
                    score * 100.0,
                    function_location(left_path, &left.definition),
                    function_location(right_path, &right.definition)
                );
                matches += 1;
            }
        }
    }
    Ok(matches)
}

fn function_location(
    path: &Path,
    function: &similarity_core::language_parser::GenericFunctionDef,
) -> String {
    let receiver = function.class_name.as_ref().map(|name| format!("{name}.")).unwrap_or_default();
    format!("{}:{} {}{}", path.display(), function.start_line, receiver, function.name)
}

fn extract_function_body(lines: &[&str], start_line: u32, end_line: u32) -> String {
    let start_idx = (start_line.saturating_sub(1)) as usize;
    let end_idx = std::cmp::min(end_line as usize, lines.len());

    if start_idx >= lines.len() {
        return String::new();
    }

    lines[start_idx..end_idx].join("\n")
}

fn check_overlaps(
    path: PathBuf,
    mut parser: GenericTreeSitterParser,
    threshold: f64,
    min_window_size: u32,
    max_window_size: u32,
    size_tolerance: f64,
) -> anyhow::Result<()> {
    use similarity_core::{find_overlaps_across_files_generic, OverlapOptions};
    use std::collections::HashMap;

    println!("Checking for overlapping code...\n");

    // Read file content
    let content = fs::read_to_string(&path)?;
    let filename = path.to_string_lossy().to_string();

    // Create file contents map
    let mut file_contents = HashMap::new();
    file_contents.insert(filename.clone(), content.clone());

    // Set up overlap options
    let options = OverlapOptions { min_window_size, max_window_size, threshold, size_tolerance };

    // Find overlaps
    let overlaps = find_overlaps_across_files_generic(&mut parser, &file_contents, &options)
        .map_err(|e| anyhow::anyhow!("Failed to find overlaps: {}", e))?;

    if overlaps.is_empty() {
        println!("\nNo code overlaps found!");
    } else {
        println!("\nCode overlaps found:");
        println!("{}", "-".repeat(60));

        for overlap_with_files in &overlaps {
            let overlap = &overlap_with_files.overlap;

            println!(
                "\nSimilarity: {:.2}% | {} nodes | {}",
                overlap.similarity * 100.0,
                overlap.node_count,
                overlap.node_type
            );
            println!(
                "  L{}-{} in function: {}",
                overlap.source_lines.0, overlap.source_lines.1, overlap.source_function
            );
            println!(
                "  L{}-{} in function: {}",
                overlap.target_lines.0, overlap.target_lines.1, overlap.target_function
            );

            // Extract and display the overlapping code
            println!("\n\x1b[36m--- Source Code ---\x1b[0m");
            if let Ok(source_segment) =
                extract_code_lines(&content, overlap.source_lines.0, overlap.source_lines.1)
            {
                println!("{source_segment}");
            }

            println!("\n\x1b[36m--- Target Code ---\x1b[0m");
            if let Ok(target_segment) =
                extract_code_lines(&content, overlap.target_lines.0, overlap.target_lines.1)
            {
                println!("{target_segment}");
            }
        }

        println!("\nTotal overlaps found: {}", overlaps.len());
    }

    Ok(())
}

fn extract_code_lines(code: &str, start_line: u32, end_line: u32) -> Result<String, String> {
    let lines: Vec<_> = code.lines().collect();

    if start_line as usize > lines.len() || end_line as usize > lines.len() {
        return Err("Line numbers out of bounds".to_string());
    }

    let start = (start_line as usize).saturating_sub(1);
    let end = (end_line as usize).min(lines.len());

    Ok(lines[start..end].join("\n"))
}
