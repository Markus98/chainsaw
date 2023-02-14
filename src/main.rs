#[macro_use]
extern crate chainsaw;
extern crate term_size;

use std::io::BufRead;
use std::{collections::HashSet, io::BufReader};
use std::fs::{File, self};
use std::path::PathBuf;

use anyhow::{Context, Result};
use bytesize::ByteSize;
use chrono::NaiveDateTime;
use chrono_tz::Tz;

use clap::{Parser, Subcommand, ArgGroup};

use chainsaw::{
    cli, get_files, lint as lint_rule, load as load_rule, set_writer, Filter, Format, Hunter,
    RuleKind, RuleLevel, RuleStatus, Searcher, Writer, ShimcacheAnalyzer,
};

#[derive(Parser)]
#[clap(
    name = "chainsaw",
    about = "Rapidly Search and Hunt through Windows Forensic Artefacts",
    after_help = r"Examples:

    Hunt with Sigma and Chainsaw Rules:
        ./chainsaw hunt evtx_attack_samples/ -s sigma/ --mapping mappings/sigma-event-logs-all.yml -r rules/

    Hunt with Sigma rules and output in JSON:
        ./chainsaw hunt evtx_attack_samples/ -s sigma/ --mapping mappings/sigma-event-logs-all.yml --json

    Search for the case-insensitive word 'mimikatz':
        ./chainsaw search mimikatz -i evtx_attack_samples/

    Search for Powershell Script Block Events (EventID 4014):
        ./chainsaw search -t 'Event.System.EventID: =4104' evtx_attack_samples/
    ",
    version
)]
struct Args {
    /// Hide Chainsaw's banner.
    #[arg(long)]
    no_banner: bool,
    /// Limit the thread number (default: num of CPUs)
    #[arg(long)]
    num_threads: Option<usize>,
    #[command(subcommand)]
    cmd: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Hunt through event logs using detection rules for threat detection
    Hunt {
        /// The path to a collection of rules to use for hunting.
        rules: Option<PathBuf>,

        /// The paths containing files to load and hunt through.
        path: Vec<PathBuf>,

        /// A mapping file to tell Chainsaw how to use third-party rules.
        #[arg(short = 'm', long = "mapping", number_of_values = 1)]
        mapping: Option<Vec<PathBuf>>,
        /// A path containing additional rules to hunt with.
        #[arg(short = 'r', long = "rule", number_of_values = 1)]
        rule: Option<Vec<PathBuf>>,

        /// Set the column width for the tabular output.
        #[arg(long = "column-width", conflicts_with = "json")]
        column_width: Option<u32>,
        /// Print the output in csv format.
        #[arg(group = "format", long = "csv", requires("output"))]
        csv: bool,
        /// Only hunt through files with the provided extension.
        #[arg(long = "extension", number_of_values = 1)]
        extension: Option<Vec<String>>,
        /// The timestamp to hunt from. Drops any documents older than the value provided.
        /// (YYYY-MM-ddTHH:mm:SS)
        #[arg(long = "from")]
        from: Option<NaiveDateTime>,
        /// Print the full values for the tabular output.
        #[arg(long = "full", conflicts_with = "json")]
        full: bool,
        /// Print the output in json format.
        #[arg(group = "format", short = 'j', long = "json")]
        json: bool,
        /// Print the output in jsonl format.
        #[arg(group = "format", long = "jsonl")]
        jsonl: bool,
        /// Restrict loaded rules to specified kinds.
        #[arg(long = "kind", number_of_values = 1)]
        kind: Vec<RuleKind>,
        /// Restrict loaded rules to specified levels.
        #[arg(long = "level", number_of_values = 1)]
        level: Vec<RuleLevel>,
        /// Allow chainsaw to try and load files it cannot identify.
        #[arg(long = "load-unknown")]
        load_unknown: bool,
        /// Output the timestamp using the local machine's timestamp.
        #[arg(long = "local", group = "tz")]
        local: bool,
        /// Display additional metadata in the tablar output.
        #[arg(long = "metadata", conflicts_with = "json")]
        metadata: bool,
        /// A path to output results to.
        #[arg(short = 'o', long = "output")]
        output: Option<PathBuf>,
        /// Print the output in log like format.
        #[arg(group = "format", long = "log")]
        log: bool,
        /// Enable preprocessing, which can result in increased performance.
        #[arg(long = "preprocess")]
        preprocess: bool,
        /// Supress informational output.
        #[arg(short = 'q')]
        quiet: bool,
        /// A path containing Sigma rules to hunt with.
        #[arg(short = 's', long = "sigma", number_of_values = 1, requires("mapping"))]
        sigma: Option<Vec<PathBuf>>,
        /// Continue to hunt when an error is encountered.
        #[arg(long = "skip-errors")]
        skip_errors: bool,
        /// Restrict loaded rules to specified statuses.
        #[arg(long = "status", number_of_values = 1)]
        status: Vec<RuleStatus>,
        /// Output the timestamp using the timezone provided.
        #[arg(long = "timezone", group = "tz")]
        timezone: Option<Tz>,
        /// The timestamp to hunt up to. Drops any documents newer than the value provided.
        /// (YYYY-MM-ddTHH:mm:SS)
        #[arg(long = "to")]
        to: Option<NaiveDateTime>,
    },

    /// Lint provided rules to ensure that they load correctly
    Lint {
        /// The path to a collection of rules.
        path: PathBuf,
        /// The kind of rule to lint: chainsaw, sigma or stalker
        #[arg(long = "kind")]
        kind: RuleKind,
        /// Output tau logic.
        #[arg(short = 't', long = "tau")]
        tau: bool,
    },

    /// Search through forensic artefacts for keywords
    Search {
        /// A string or regular expression pattern to search for.
        /// Not used when -e or -t is specified.
        #[arg(required_unless_present_any=&["additional_pattern", "tau"])]
        pattern: Option<String>,

        /// The paths containing files to load and hunt through.
        path: Vec<PathBuf>,

        /// A string or regular expression pattern to search for.
        #[arg(
            short = 'e',
            long = "regex",
            value_name = "pattern",
            number_of_values = 1
        )]
        additional_pattern: Option<Vec<String>>,

        /// Only search through files with the provided extension.
        #[arg(long = "extension", number_of_values = 1)]
        extension: Option<Vec<String>>,
        /// The timestamp to search from. Drops any documents older than the value provided.
        /// (YYYY-MM-ddTHH:mm:SS)
        #[arg(long = "from", requires = "timestamp")]
        from: Option<NaiveDateTime>,
        /// Ignore the case when searching patterns
        #[arg(short = 'i', long = "ignore-case")]
        ignore_case: bool,
        /// Print the output in json format.
        #[arg(short = 'j', long = "json")]
        json: bool,
        /// Print the output in jsonl format.
        #[arg(group = "format", long = "jsonl")]
        jsonl: bool,
        /// Allow chainsaw to try and load files it cannot identify.
        #[arg(long = "load-unknown")]
        load_unknown: bool,
        /// Output the timestamp using the local machine's timestamp.
        #[arg(long = "local", group = "tz")]
        local: bool,
        /// The path to output results to.
        #[arg(short = 'o', long = "output")]
        output: Option<PathBuf>,
        /// Supress informational output.
        #[arg(short = 'q')]
        quiet: bool,
        /// Continue to search when an error is encountered.
        #[arg(long = "skip-errors")]
        skip_errors: bool,
        /// Tau expressions to search with. e.g. 'Event.System.EventID: =4104'
        #[arg(short = 't', long = "tau", number_of_values = 1)]
        tau: Option<Vec<String>>,
        /// The field that contains the timestamp.
        #[arg(long = "timestamp")]
        timestamp: Option<String>,
        /// Output the timestamp using the timezone provided.
        #[arg(long = "timezone", group = "tz")]
        timezone: Option<Tz>,
        /// The timestamp to search up to. Drops any documents newer than the value provided.
        /// (YYYY-MM-ddTHH:mm:SS)
        #[arg(long = "to", requires = "timestamp")]
        to: Option<NaiveDateTime>,
    },

    /// Perform various analyses on artifacts
    Analyse {
        #[command(subcommand)]
        cmd: AnalyseCommand,
    },
}

#[derive(Subcommand)]
enum AnalyseCommand {
    /// Create an execution timeline from the shimcache by detecting executables that are compiled before execution
    #[clap(group(
        ArgGroup::new("regex")
            .multiple(true)
            .required(true)
            .args(&["additional_pattern", "regex_file"]),
    ))]
    Shimcache {
        /// The path to the shimcache artifact (SYSTEM registry file)
        shimcache: PathBuf,
        /// A string or regular expression pattern to search for
        #[arg(
            short = 'e',
            long = "regex",
            value_name = "pattern",
            number_of_values = 1
        )]
        additional_pattern: Option<Vec<String>>,
        /// The path to the newline delimited file containing regex patterns to match
        #[arg(short = 'r', long = "regexfile")]
        regex_file: Option<PathBuf>,
        /// A path to output the resulting csv file
        #[arg(short = 'o', long = "output")]
        output: Option<PathBuf>,
        /// The path to the amcache artifact (Amcache.hve) for timeline enrichment
        #[arg(short = 'a', long = "amcache")]
        amcache: Option<PathBuf>,
    }
}

fn print_title() {
    cs_eprintln!(
        "
 ██████╗██╗  ██╗ █████╗ ██╗███╗   ██╗███████╗ █████╗ ██╗    ██╗
██╔════╝██║  ██║██╔══██╗██║████╗  ██║██╔════╝██╔══██╗██║    ██║
██║     ███████║███████║██║██╔██╗ ██║███████╗███████║██║ █╗ ██║
██║     ██╔══██║██╔══██║██║██║╚██╗██║╚════██║██╔══██║██║███╗██║
╚██████╗██║  ██║██║  ██║██║██║ ╚████║███████║██║  ██║╚███╔███╔╝
 ╚═════╝╚═╝  ╚═╝╚═╝  ╚═╝╚═╝╚═╝  ╚═══╝╚══════╝╚═╝  ╚═╝ ╚══╝╚══╝
    By Countercept (@FranticTyping, @AlexKornitzer)
"
    );
}

fn resolve_col_width() -> Option<u32> {
    // Get windows size and return a rough mapping for sutiable col width
    match term_size::dimensions() {
        Some((w, _h)) => match w {
            50..=120 => Some(20),
            121..=239 => Some(30),
            240..=340 => Some(50),
            341..=430 => Some(90),
            431..=550 => Some(130),
            551.. => Some(160),
            _ => None,
        },
        None => None,
    }
}

fn init_writer(output: Option<PathBuf>, csv: bool, json: bool, quiet: bool) -> crate::Result<()> {
    let (path, output) = match &output {
        Some(path) => {
            if csv {
                (Some(path.to_path_buf()), None)
            } else {
                let file = match File::create(path) {
                    Ok(f) => f,
                    Err(e) => {
                        return Err(anyhow::anyhow!(
                            "Unable to write to specified output file - {} - {}",
                            path.display(),
                            e
                        ));
                    }
                };
                (None, Some(file))
            }
        }
        None => (None, None),
    };
    let format = if csv {
        Format::Csv
    } else if json {
        Format::Json
    } else {
        Format::Std
    };
    let writer = Writer {
        format,
        output,
        path,
        quiet,
    };
    set_writer(writer).expect("could not set writer");
    Ok(())
}

fn run() -> Result<()> {
    let args = Args::parse();
    if let Some(num_threads) = args.num_threads {
        let _ = rayon::ThreadPoolBuilder::new()
            .num_threads(num_threads)
            .build_global()?;
    }
    match args.cmd {
        Command::Hunt {
            rules,
            mut path,

            mapping,
            rule,

            load_unknown,
            mut column_width,
            csv,
            extension,
            from,
            full,
            json,
            jsonl,
            kind,
            level,
            local,
            metadata,
            output,
            log,
            preprocess,
            quiet,
            sigma,
            skip_errors,
            status,
            timezone,
            to,
        } => {
            if column_width.is_none() {
                column_width = resolve_col_width();
            }
            init_writer(output.clone(), csv, json, quiet)?;
            if !args.no_banner {
                print_title();
            }
            let mut rs = vec![];
            if rule.is_some() || sigma.is_some() {
                if let Some(rules) = rules {
                    let mut paths = vec![rules];
                    paths.extend(path);
                    path = paths;
                }
            } else if let Some(rules) = rules {
                rs = vec![rules];
            }
            let mut rules = rs;
            if let Some(rule) = rule {
                rules.extend(rule)
            };
            let sigma = sigma.unwrap_or_default();

            cs_eprintln!(
                "[+] Loading detection rules from: {}",
                rules
                    .iter()
                    .chain(sigma.iter())
                    .map(|r| r.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            let kinds: Option<HashSet<RuleKind>> = if kind.is_empty() {
                None
            } else {
                Some(HashSet::from_iter(kind.into_iter()))
            };
            let levels: Option<HashSet<RuleLevel>> = if level.is_empty() {
                None
            } else {
                Some(HashSet::from_iter(level.into_iter()))
            };
            let statuses: Option<HashSet<RuleStatus>> = if status.is_empty() {
                None
            } else {
                Some(HashSet::from_iter(status.into_iter()))
            };
            let mut failed = 0;
            let mut count = 0;
            let mut rs = vec![];
            for path in &rules {
                for file in get_files(path, &None, skip_errors)? {
                    match load_rule(RuleKind::Chainsaw, &file, &kinds, &levels, &statuses) {
                        Ok(r) => {
                            if !r.is_empty() {
                                count += 1;
                                rs.extend(r)
                            }
                        }
                        Err(_) => {
                            failed += 1;
                        }
                    }
                }
            }
            for path in &sigma {
                for file in get_files(path, &None, skip_errors)? {
                    match load_rule(RuleKind::Sigma, &file, &kinds, &levels, &statuses) {
                        Ok(r) => {
                            if !r.is_empty() {
                                count += 1;
                                rs.extend(r)
                            }
                        }
                        Err(_) => {
                            failed += 1;
                        }
                    }
                }
            }
            if failed > 500 && sigma.is_empty() {
                cs_eyellowln!("[!] {} rules failed to load, ensure Sigma rule paths are specified with the '-s' flag", failed);
            }
            if count == 0 {
                return Err(anyhow::anyhow!(
                    "No valid detection rules were found in the provided paths",
                ));
            }
            if failed > 0 {
                cs_eprintln!(
                    "[+] Loaded {} detection rules ({} not loaded)",
                    count,
                    failed
                );
            } else {
                cs_eprintln!("[+] Loaded {} detection rules", count);
            }

            let rules = rs;
            let mut hunter = Hunter::builder()
                .rules(rules)
                .mappings(mapping.unwrap_or_default())
                .load_unknown(load_unknown)
                .local(local)
                .preprocess(preprocess)
                .skip_errors(skip_errors);
            if let Some(from) = from {
                hunter = hunter.from(from);
            }
            if let Some(timezone) = timezone {
                hunter = hunter.timezone(timezone);
            }
            if let Some(to) = to {
                hunter = hunter.to(to);
            }
            let hunter = hunter.build()?;

            /* if no user-defined extensions are specified, then we parse rules and
            mappings to build a list of file extensions that should be loaded */
            let mut scratch = HashSet::new();
            let message;
            let exts = if load_unknown {
                message = "*".to_string();
                None
            } else {
                scratch.extend(hunter.extensions());
                if scratch.is_empty() {
                    return Err(anyhow::anyhow!(
                        "No valid file extensions for the 'kind' specified in the mapping or rules files"
                    ));
                }
                if let Some(e) = extension {
                    // User has provided specific extensions
                    scratch = scratch
                        .intersection(&HashSet::from_iter(e.iter().cloned()))
                        .cloned()
                        .collect();
                    if scratch.is_empty() {
                        return Err(anyhow::anyhow!(
                        "The specified file extension is not supported. Use --load-unknown to force loading",
                    ));
                    }
                };
                message = scratch
                    .iter()
                    .map(|x| format!(".{}", x))
                    .collect::<Vec<_>>()
                    .join(", ");
                Some(scratch)
            };

            cs_eprintln!(
                "[+] Loading forensic artefacts from: {} (extensions: {})",
                path.iter()
                    .map(|p| p.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", "),
                message
            );

            let mut files = vec![];
            let mut size = ByteSize::mb(0);
            for path in &path {
                let res = get_files(path, &exts, skip_errors)?;
                for i in &res {
                    size += i.metadata()?.len();
                }
                files.extend(res);
            }
            if files.is_empty() {
                return Err(anyhow::anyhow!(
                    "No compatible files were found in the provided paths",
                ));
            } else {
                cs_eprintln!("[+] Loaded {} forensic artefacts ({})", files.len(), size);
            }
            let mut detections = vec![];
            let pb = cli::init_progress_bar(files.len() as u64, "Hunting".to_string());
            for file in &files {
                pb.tick();
                detections.extend(hunter.hunt(file).with_context(|| {
                    format!("Failed to hunt through file '{}'", file.to_string_lossy())
                })?);
                pb.inc(1);
            }
            pb.finish();
            if csv {
                cli::print_csv(&detections, hunter.hunts(), hunter.rules(), local, timezone)?;
            } else if json || jsonl {
                if output.is_some() {
                    cs_eprintln!("[+] Writing results to output file...");
                }
                cli::print_json(
                    &detections,
                    hunter.hunts(),
                    hunter.rules(),
                    local,
                    timezone,
                    jsonl,
                )?;
            } else if log {
                cli::print_log(&detections, hunter.hunts(), hunter.rules(), local, timezone)?;
            } else {
                cli::print_detections(
                    &detections,
                    hunter.hunts(),
                    hunter.rules(),
                    column_width.unwrap_or(40),
                    full,
                    local,
                    metadata,
                    timezone,
                );
            }
            cs_eprintln!(
                "[+] {} Detections found on {} documents",
                detections.iter().map(|d| d.hits.len()).sum::<usize>(),
                detections.len()
            );
        }
        Command::Lint { path, kind, tau } => {
            init_writer(None, false, false, false)?;
            if !args.no_banner {
                print_title();
            }
            cs_eprintln!("[+] Validating as {} for supplied detection rules...", kind);
            let mut count = 0;
            let mut failed = 0;
            for file in get_files(&path, &None, false)? {
                match lint_rule(&kind, &file) {
                    Ok(filters) => {
                        if tau {
                            cs_eprintln!("[+] Rule {}:", file.to_string_lossy());
                            for filter in filters {
                                let yaml = match filter {
                                    Filter::Detection(mut d) => {
                                        d.expression = tau_engine::core::optimiser::coalesce(
                                            d.expression,
                                            &d.identifiers,
                                        );
                                        d.identifiers.clear();
                                        d.expression =
                                            tau_engine::core::optimiser::shake(d.expression);
                                        d.expression =
                                            tau_engine::core::optimiser::rewrite(d.expression);
                                        d.expression =
                                            tau_engine::core::optimiser::matrix(d.expression);
                                        serde_yaml::to_string(&d)?
                                    }
                                    Filter::Expression(_) => {
                                        cs_eyellowln!("[!] Tau does not support visual representation of expressions");
                                        continue;
                                    }
                                };
                                println!("{}", yaml);
                            }
                        }
                    }
                    Err(e) => {
                        failed += 1;
                        let file_name = match file
                            .display()
                            .to_string()
                            .strip_prefix(&path.display().to_string())
                        {
                            Some(e) => e.to_string(),
                            None => file.display().to_string(),
                        };
                        cs_eprintln!("[!] {}: {}", file_name, e);
                        continue;
                    }
                }
                count += 1;
            }
            cs_eprintln!(
                "[+] Validated {} detection rules out of {}",
                count,
                count + failed
            );
        }
        Command::Search {
            path,

            mut pattern,
            additional_pattern,

            extension,
            from,
            ignore_case,
            json,
            jsonl,
            load_unknown,
            local,
            output,
            quiet,
            skip_errors,
            tau,
            timestamp,
            timezone,
            to,
        } => {
            init_writer(output, false, json, quiet)?;
            if !args.no_banner {
                print_title();
            }
            let mut paths = if additional_pattern.is_some() || tau.is_some() {
                let mut scratch = pattern
                    .take()
                    .map(|p| vec![PathBuf::from(p)])
                    .unwrap_or_default();
                scratch.extend(path);
                scratch
            } else {
                path
            };
            if paths.is_empty() {
                paths.push(
                    std::env::current_dir().expect("could not get current working directory"),
                );
            }

            let types = extension.as_ref().map(|e| HashSet::from_iter(e.clone()));
            let mut files = vec![];
            let mut size = ByteSize::mb(0);
            for path in &paths {
                let res = get_files(path, &types, skip_errors)?;
                for i in &res {
                    size += i.metadata()?.len();
                }
                files.extend(res);
            }
            if let Some(ext) = &extension {
                cs_eprintln!(
                    "[+] Loading forensic artefacts from: {} (extensions: {})",
                    paths
                        .iter()
                        .map(|p| p.display().to_string())
                        .collect::<Vec<_>>()
                        .join(", "),
                    ext.iter()
                        .map(|x| format!(".{}", x))
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            } else {
                cs_eprintln!(
                    "[+] Loading forensic artefacts from: {}",
                    paths
                        .iter()
                        .map(|p| p.display().to_string())
                        .collect::<Vec<_>>()
                        .join(", "),
                )
            };

            if files.is_empty() {
                return Err(anyhow::anyhow!(
                    "No forensic artefacts were found in the provided paths",
                ));
            } else {
                cs_eprintln!("[+] Loaded {} forensic files ({})", files.len(), size);
            }
            let mut searcher = Searcher::builder()
                .ignore_case(ignore_case)
                .load_unknown(load_unknown)
                .local(local)
                .skip_errors(skip_errors);
            if let Some(patterns) = additional_pattern {
                searcher = searcher.patterns(patterns);
            } else if let Some(pattern) = pattern {
                searcher = searcher.patterns(vec![pattern]);
            }
            if let Some(from) = from {
                searcher = searcher.from(from);
            }
            if let Some(tau) = tau {
                searcher = searcher.tau(tau);
            }
            if let Some(timestamp) = timestamp {
                searcher = searcher.timestamp(timestamp);
            }
            if let Some(timezone) = timezone {
                searcher = searcher.timezone(timezone);
            }
            if let Some(to) = to {
                searcher = searcher.to(to);
            }
            let searcher = searcher.build()?;
            cs_eprintln!("[+] Searching forensic artefacts...");
            if json {
                cs_print!("[");
            }
            let mut hits = 0;
            for file in &files {
                for res in searcher.search(file)?.iter() {
                    let hit = match res {
                        Ok(hit) => hit,
                        Err(e) => {
                            if skip_errors {
                                continue;
                            }
                            anyhow::bail!("Failed to search file... - {}", e);
                        }
                    };
                    if json {
                        if hits != 0 {
                            cs_print!(",");
                        }
                        cs_print_json!(&hit)?;
                    } else if jsonl {
                        cs_print_json!(&hit)?;
                        println!();
                    } else {
                        cs_print_yaml!(&hit)?;
                    }
                    hits += 1;
                }
            }
            if json {
                cs_println!("]");
            }
            cs_eprintln!("[+] Found {} hits", hits);
        }
        Command::Analyse {
            cmd,
        } => {
            match cmd {
                AnalyseCommand::Shimcache {
                    additional_pattern,
                    amcache,
                    output,
                    regex_file,
                    shimcache,
                } => {
                    if !args.no_banner {
                        print_title();
                    }
                    init_writer(output.clone(), true, false, false)?;
                    let shimcache_analyzer = ShimcacheAnalyzer::new(shimcache, amcache);

                    // Load regex
                    let mut regex_patterns: Vec<String> = Vec::new();
                    if let Some(regex_file) = regex_file {
                        let mut file_regex_patterns = BufReader::new(File::open(&regex_file)?)
                            .lines().collect::<Result<Vec<_>, _>>()?;
                        cs_eprintln!("[+] Regex file with {} pattern(s) loaded from {:?}", 
                            file_regex_patterns.len(),
                            fs::canonicalize(&regex_file).expect("cloud not get absolute path")
                        );
                        regex_patterns.append(&mut file_regex_patterns);
                    }
                    if let Some(mut additional_patterns) = additional_pattern {
                        regex_patterns.append(&mut additional_patterns);
                    }

                    // Do analysis
                    let timeline = shimcache_analyzer.amcache_shimcache_timeline(&regex_patterns)?;
                    if let Some(entities) = timeline {
                        cli::print_shimcache_analysis_csv(&entities)?;
                        if let Some(output_path) = output {
                            cs_eprintln!("[+] Saved output to {:?}", std::fs::canonicalize(output_path)
                                .expect("could not get absolute path"));
                        }
                    } else {
                        cs_eyellowln!("[!] No matching entries found from shimcache, nothing to output")
                    }
                }
            }
        }
    }
    Ok(())
}

fn main() {
    if let Err(e) = run() {
        if let Some(cause) = e.chain().skip(1).next() {
            cs_eredln!("[x] {} - {}", e, cause);
        } else {
            cs_eredln!("[x] {}", e);
        }
        std::process::exit(1);
    }
}
