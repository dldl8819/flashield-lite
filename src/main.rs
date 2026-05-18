mod flash;
mod lru;
mod metrics;
mod policy;
mod trace;

use std::env;
use std::error::Error;
use std::fmt::Write as _;
use std::fs::{self, File};
use std::io::{BufWriter, Write as _};
use std::path::PathBuf;

use metrics::Metrics;
use policy::{CachePolicy, DramLruCache, FlashieldConfig, FlashieldLiteCache, NaiveFlashCache};

const DEFAULT_DRAM_CAPACITY: u64 = 1024 * 1024;
const DEFAULT_FLASH_CAPACITY: u64 = 10 * 1024 * 1024;
const DEFAULT_SEGMENT_SIZE: u64 = 1024 * 1024;
const DEFAULT_MIN_READS: u64 = 2;
const DEFAULT_MAX_UPDATES: u64 = 1;
const DEFAULT_MIN_AGE: u64 = 2;

fn main() {
    if let Err(err) = run(env::args().collect()) {
        eprintln!("error: {err}");
        eprintln!();
        eprintln!("{}", usage());
        std::process::exit(1);
    }
}

fn run(args: Vec<String>) -> Result<(), Box<dyn Error>> {
    let Some(command) = args.get(1).map(String::as_str) else {
        return Err("missing command".into());
    };

    match command {
        "simulate" => simulate(&args[2..]),
        "generate-trace" => generate_trace(&args[2..]),
        "--help" | "-h" | "help" => {
            println!("{}", usage());
            Ok(())
        }
        other => Err(format!("unknown command: {other}").into()),
    }
}

fn simulate(args: &[String]) -> Result<(), Box<dyn Error>> {
    let options = SimulateOptions::parse(args)?;
    let events = trace::read_trace(&options.trace)?;
    let mut metrics = Metrics::default();

    match options.policy.as_str() {
        "dram-lru" => {
            let mut cache = DramLruCache::new(options.dram_capacity);
            for event in &events {
                cache.handle(event, &mut metrics);
            }
            cache.finish(&mut metrics);
        }
        "naive-flash" => {
            let mut cache = NaiveFlashCache::new(
                options.dram_capacity,
                options.flash_capacity,
                options.segment_size,
            );
            for event in &events {
                cache.handle(event, &mut metrics);
            }
            cache.finish(&mut metrics);
        }
        "flashield-lite" => {
            let config = FlashieldConfig {
                dram_capacity: options.dram_capacity,
                flash_capacity: options.flash_capacity,
                segment_size: options.segment_size,
                min_reads: options.min_reads,
                max_updates: options.max_updates,
                min_age: options.min_age,
            };
            let mut cache = FlashieldLiteCache::new(config);
            for event in &events {
                cache.handle(event, &mut metrics);
            }
            cache.finish(&mut metrics);
        }
        other => return Err(format!("unknown policy: {other}").into()),
    }

    match options.output_format {
        OutputFormat::Text => print!("{}", format_text_report(&options.policy, &metrics)),
        OutputFormat::Json => print!("{}", format_json_report(&options.policy, &metrics)),
    }
    Ok(())
}

fn format_text_report(policy: &str, metrics: &Metrics) -> String {
    let mut report = String::new();
    writeln!(&mut report, "Policy: {policy}").expect("write to String should not fail");
    writeln!(&mut report, "Total requests: {}", metrics.total_requests)
        .expect("write to String should not fail");
    writeln!(&mut report, "Lookup requests: {}", metrics.lookup_requests)
        .expect("write to String should not fail");
    writeln!(&mut report, "Cache hits: {}", metrics.cache_hits)
        .expect("write to String should not fail");
    writeln!(&mut report, "Cache misses: {}", metrics.cache_misses)
        .expect("write to String should not fail");
    writeln!(&mut report, "Hit rate: {:.2}%", metrics.hit_rate() * 100.0)
        .expect("write to String should not fail");
    writeln!(&mut report, "DRAM hits: {}", metrics.dram_hits)
        .expect("write to String should not fail");
    writeln!(&mut report, "Flash hits: {}", metrics.flash_hits)
        .expect("write to String should not fail");
    writeln!(
        &mut report,
        "Flash bytes written: {}",
        metrics.flash_bytes_written
    )
    .expect("write to String should not fail");
    writeln!(
        &mut report,
        "Logical bytes admitted: {}",
        metrics.logical_bytes_admitted
    )
    .expect("write to String should not fail");
    writeln!(
        &mut report,
        "Write amplification: {:.2}",
        metrics.write_amplification()
    )
    .expect("write to String should not fail");
    writeln!(&mut report, "Segment flushes: {}", metrics.segment_flushes)
        .expect("write to String should not fail");
    writeln!(&mut report, "Evictions: {}", metrics.evictions)
        .expect("write to String should not fail");
    report
}

fn format_json_report(policy: &str, metrics: &Metrics) -> String {
    format!(
        concat!(
            "{{\n",
            "  \"policy\": \"{}\",\n",
            "  \"total_requests\": {},\n",
            "  \"lookup_requests\": {},\n",
            "  \"cache_hits\": {},\n",
            "  \"cache_misses\": {},\n",
            "  \"hit_rate\": {:.6},\n",
            "  \"dram_hits\": {},\n",
            "  \"flash_hits\": {},\n",
            "  \"flash_bytes_written\": {},\n",
            "  \"logical_bytes_admitted\": {},\n",
            "  \"write_amplification\": {:.6},\n",
            "  \"segment_flushes\": {},\n",
            "  \"evictions\": {}\n",
            "}}\n"
        ),
        escape_json_string(policy),
        metrics.total_requests,
        metrics.lookup_requests,
        metrics.cache_hits,
        metrics.cache_misses,
        metrics.hit_rate(),
        metrics.dram_hits,
        metrics.flash_hits,
        metrics.flash_bytes_written,
        metrics.logical_bytes_admitted,
        metrics.write_amplification(),
        metrics.segment_flushes,
        metrics.evictions
    )
}

fn escape_json_string(value: &str) -> String {
    let mut escaped = String::new();
    for character in value.chars() {
        match character {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            character if character.is_control() => {
                write!(&mut escaped, "\\u{:04x}", character as u32)
                    .expect("write to String should not fail");
            }
            character => escaped.push(character),
        }
    }
    escaped
}

fn generate_trace(args: &[String]) -> Result<(), Box<dyn Error>> {
    let options = GenerateOptions::parse(args)?;
    if let Some(parent) = options
        .output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)?;
    }

    let file = File::create(&options.output)?;
    let mut writer = BufWriter::new(file);
    let mut rng = Lcg::new(0x5eed_1234_5678_9abc);
    let key_count = options.keys.max(1);
    let mut known = vec![false; key_count as usize];

    writeln!(writer, "timestamp,op,key,size")?;
    for timestamp in 1..=options.requests {
        let key_idx = rng.range(key_count);
        let key = format!("synthetic:{key_idx}");
        let roll = rng.range(100);
        let known_key = known[key_idx as usize];
        let op = options.preset.choose_op(known_key, roll);

        let size = match op {
            GeneratedOp::Set => {
                known[key_idx as usize] = true;
                synthetic_size(&mut rng)
            }
            GeneratedOp::Get => 0,
            GeneratedOp::Update => synthetic_size(&mut rng),
            GeneratedOp::Delete => {
                known[key_idx as usize] = false;
                0
            }
        };

        let op_name = op.as_str();
        writeln!(writer, "{timestamp},{op_name},{key},{size}")?;
    }

    writer.flush()?;
    println!(
        "Generated {} synthetic requests with the {} preset.",
        options.requests,
        options.preset.as_str()
    );
    Ok(())
}

fn synthetic_size(rng: &mut Lcg) -> u64 {
    match rng.range(10) {
        0 => 64,
        1 | 2 => 128,
        3 | 4 => 256,
        5 | 6 => 512,
        7 | 8 => 1024,
        _ => 4096,
    }
}

#[derive(Debug)]
struct SimulateOptions {
    policy: String,
    trace: PathBuf,
    output_format: OutputFormat,
    dram_capacity: u64,
    flash_capacity: u64,
    segment_size: u64,
    min_reads: u64,
    max_updates: u64,
    min_age: u64,
}

impl SimulateOptions {
    fn parse(args: &[String]) -> Result<Self, Box<dyn Error>> {
        let mut parser = ArgParser::new(args);
        let policy = parser.required("--policy")?;
        let trace = PathBuf::from(parser.required("--trace")?);
        let output_format = match parser.optional("--output-format")? {
            Some(value) => OutputFormat::parse(&value)?,
            None => OutputFormat::Text,
        };
        let dram_capacity = parser
            .optional_u64("--dram-capacity")?
            .unwrap_or(DEFAULT_DRAM_CAPACITY);
        let flash_capacity = parser
            .optional_u64("--flash-capacity")?
            .unwrap_or(DEFAULT_FLASH_CAPACITY);
        let segment_size = parser
            .optional_u64("--segment-size")?
            .unwrap_or(DEFAULT_SEGMENT_SIZE);
        let min_reads = parser
            .optional_u64("--min-reads")?
            .unwrap_or(DEFAULT_MIN_READS);
        let max_updates = parser
            .optional_u64("--max-updates")?
            .unwrap_or(DEFAULT_MAX_UPDATES);
        let min_age = parser.optional_u64("--min-age")?.unwrap_or(DEFAULT_MIN_AGE);
        parser.finish()?;

        if segment_size == 0 {
            return Err("--segment-size must be greater than zero".into());
        }

        Ok(Self {
            policy,
            trace,
            output_format,
            dram_capacity,
            flash_capacity,
            segment_size,
            min_reads,
            max_updates,
            min_age,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OutputFormat {
    Text,
    Json,
}

impl OutputFormat {
    fn parse(value: &str) -> Result<Self, Box<dyn Error>> {
        match value {
            "text" => Ok(Self::Text),
            "json" => Ok(Self::Json),
            other => Err(format!("unsupported output format: {other}").into()),
        }
    }
}

#[derive(Debug)]
struct GenerateOptions {
    output: PathBuf,
    requests: u64,
    keys: u64,
    preset: WorkloadPreset,
}

impl GenerateOptions {
    fn parse(args: &[String]) -> Result<Self, Box<dyn Error>> {
        let mut parser = ArgParser::new(args);
        let output = PathBuf::from(parser.required("--output")?);
        let requests = parser.optional_u64("--requests")?.unwrap_or(10_000);
        let keys = parser.optional_u64("--keys")?.unwrap_or(1_000);
        let preset = match parser.optional("--preset")? {
            Some(value) => WorkloadPreset::parse(&value)?,
            None => WorkloadPreset::Mixed,
        };
        parser.finish()?;

        Ok(Self {
            output,
            requests,
            keys,
            preset,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WorkloadPreset {
    Mixed,
    ReadHeavy,
    UpdateHeavy,
}

impl WorkloadPreset {
    fn parse(value: &str) -> Result<Self, Box<dyn Error>> {
        match value {
            "mixed" => Ok(Self::Mixed),
            "read-heavy" => Ok(Self::ReadHeavy),
            "update-heavy" => Ok(Self::UpdateHeavy),
            other => Err(format!("unsupported workload preset: {other}").into()),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Mixed => "mixed",
            Self::ReadHeavy => "read-heavy",
            Self::UpdateHeavy => "update-heavy",
        }
    }

    fn choose_op(self, known_key: bool, roll: u64) -> GeneratedOp {
        if !known_key {
            return GeneratedOp::Set;
        }

        match self {
            Self::Mixed => match roll {
                0..=19 => GeneratedOp::Set,
                20..=69 => GeneratedOp::Get,
                70..=91 => GeneratedOp::Update,
                _ => GeneratedOp::Delete,
            },
            Self::ReadHeavy => match roll {
                0..=9 => GeneratedOp::Set,
                10..=84 => GeneratedOp::Get,
                85..=94 => GeneratedOp::Update,
                _ => GeneratedOp::Delete,
            },
            Self::UpdateHeavy => match roll {
                0..=14 => GeneratedOp::Set,
                15..=34 => GeneratedOp::Get,
                35..=89 => GeneratedOp::Update,
                _ => GeneratedOp::Delete,
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GeneratedOp {
    Get,
    Set,
    Update,
    Delete,
}

impl GeneratedOp {
    fn as_str(self) -> &'static str {
        match self {
            Self::Get => "get",
            Self::Set => "set",
            Self::Update => "update",
            Self::Delete => "delete",
        }
    }
}

struct ArgParser<'a> {
    args: &'a [String],
    used: Vec<bool>,
}

impl<'a> ArgParser<'a> {
    fn new(args: &'a [String]) -> Self {
        Self {
            args,
            used: vec![false; args.len()],
        }
    }

    fn required(&mut self, flag: &str) -> Result<String, Box<dyn Error>> {
        self.optional(flag)?
            .ok_or_else(|| format!("missing required option {flag}").into())
    }

    fn optional_u64(&mut self, flag: &str) -> Result<Option<u64>, Box<dyn Error>> {
        self.optional(flag)?
            .map(|value| {
                value
                    .parse::<u64>()
                    .map_err(|_| format!("{flag} must be an unsigned integer").into())
            })
            .transpose()
    }

    fn optional(&mut self, flag: &str) -> Result<Option<String>, Box<dyn Error>> {
        for index in 0..self.args.len() {
            if self.used[index] || self.args[index] != flag {
                continue;
            }

            let value_index = index + 1;
            if value_index >= self.args.len() || self.args[value_index].starts_with("--") {
                return Err(format!("{flag} requires a value").into());
            }

            self.used[index] = true;
            self.used[value_index] = true;
            return Ok(Some(self.args[value_index].clone()));
        }

        Ok(None)
    }

    fn finish(&self) -> Result<(), Box<dyn Error>> {
        for (index, arg) in self.args.iter().enumerate() {
            if !self.used[index] {
                return Err(format!("unexpected argument: {arg}").into());
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
struct Lcg {
    state: u64,
}

impl Lcg {
    fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next(&mut self) -> u64 {
        self.state = self
            .state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.state
    }

    fn range(&mut self, upper: u64) -> u64 {
        if upper == 0 {
            0
        } else {
            self.next() % upper
        }
    }
}

fn usage() -> &'static str {
    "Usage:
  cargo run -- simulate --policy <dram-lru|naive-flash|flashield-lite> --trace <path> [options]
  cargo run -- generate-trace --output <path> [--requests <n>] [--keys <n>] [--preset <preset>]

Options:
  --dram-capacity <bytes>   DRAM cache capacity, default 1048576
  --flash-capacity <bytes>  Flash cache capacity, default 10485760
  --segment-size <bytes>    Simulated flash segment size, default 1048576
  --output-format <format>  Report format: text or json, default text
  --preset <preset>         Trace preset: mixed, read-heavy, or update-heavy, default mixed
  --min-reads <n>           Flashield-lite admission threshold, default 2
  --max-updates <n>         Flashield-lite admission threshold, default 1
  --min-age <ticks>         Flashield-lite admission threshold, default 2"
}

#[cfg(test)]
mod tests {
    use super::{format_json_report, format_text_report, GeneratedOp, WorkloadPreset};
    use crate::metrics::Metrics;

    #[test]
    fn text_report_includes_human_readable_metrics() {
        let metrics = Metrics {
            total_requests: 10,
            lookup_requests: 4,
            cache_hits: 3,
            cache_misses: 1,
            dram_hits: 2,
            flash_hits: 1,
            flash_bytes_written: 128,
            logical_bytes_admitted: 64,
            segment_flushes: 2,
            evictions: 1,
        };

        let report = format_text_report("flashield-lite", &metrics);

        assert!(report.contains("Policy: flashield-lite"));
        assert!(report.contains("Hit rate: 75.00%"));
        assert!(report.contains("Write amplification: 2.00"));
    }

    #[test]
    fn json_report_includes_machine_readable_metrics() {
        let metrics = Metrics {
            total_requests: 10,
            lookup_requests: 4,
            cache_hits: 3,
            cache_misses: 1,
            dram_hits: 2,
            flash_hits: 1,
            flash_bytes_written: 128,
            logical_bytes_admitted: 64,
            segment_flushes: 2,
            evictions: 1,
        };

        let report = format_json_report("flashield-lite", &metrics);

        assert!(report.contains("\"policy\": \"flashield-lite\""));
        assert!(report.contains("\"hit_rate\": 0.750000"));
        assert!(report.contains("\"write_amplification\": 2.000000"));
    }

    #[test]
    fn parses_workload_presets() {
        assert_eq!(
            WorkloadPreset::parse("mixed").unwrap(),
            WorkloadPreset::Mixed
        );
        assert_eq!(
            WorkloadPreset::parse("read-heavy").unwrap(),
            WorkloadPreset::ReadHeavy
        );
        assert_eq!(
            WorkloadPreset::parse("update-heavy").unwrap(),
            WorkloadPreset::UpdateHeavy
        );
        assert!(WorkloadPreset::parse("unknown").is_err());
    }

    #[test]
    fn workload_presets_choose_expected_operations() {
        assert_eq!(WorkloadPreset::Mixed.choose_op(false, 99), GeneratedOp::Set);
        assert_eq!(WorkloadPreset::Mixed.choose_op(true, 50), GeneratedOp::Get);
        assert_eq!(
            WorkloadPreset::ReadHeavy.choose_op(true, 80),
            GeneratedOp::Get
        );
        assert_eq!(
            WorkloadPreset::UpdateHeavy.choose_op(true, 80),
            GeneratedOp::Update
        );
    }
}
