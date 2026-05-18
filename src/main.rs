mod flash;
mod lru;
mod metrics;
mod policy;
mod trace;

use std::env;
use std::error::Error;
use std::fs::{self, File};
use std::io::{BufWriter, Write};
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

    print_report(&options.policy, &metrics);
    Ok(())
}

fn print_report(policy: &str, metrics: &Metrics) {
    println!("Policy: {policy}");
    println!("Total requests: {}", metrics.total_requests);
    println!("Lookup requests: {}", metrics.lookup_requests);
    println!("Cache hits: {}", metrics.cache_hits);
    println!("Cache misses: {}", metrics.cache_misses);
    println!("Hit rate: {:.2}%", metrics.hit_rate() * 100.0);
    println!("DRAM hits: {}", metrics.dram_hits);
    println!("Flash hits: {}", metrics.flash_hits);
    println!("Flash bytes written: {}", metrics.flash_bytes_written);
    println!("Logical bytes admitted: {}", metrics.logical_bytes_admitted);
    println!("Write amplification: {:.2}", metrics.write_amplification());
    println!("Segment flushes: {}", metrics.segment_flushes);
    println!("Evictions: {}", metrics.evictions);
}

fn generate_trace(args: &[String]) -> Result<(), Box<dyn Error>> {
    let options = GenerateOptions::parse(args)?;
    if let Some(parent) = options.output.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
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

        let (op, size) = if !known_key || roll < 20 {
            known[key_idx as usize] = true;
            ("set", synthetic_size(&mut rng))
        } else if roll < 70 {
            ("get", 0)
        } else if roll < 92 {
            ("update", synthetic_size(&mut rng))
        } else {
            known[key_idx as usize] = false;
            ("delete", 0)
        };

        writeln!(writer, "{timestamp},{op},{key},{size}")?;
    }

    writer.flush()?;
    println!("Generated {} synthetic requests.", options.requests);
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
            dram_capacity,
            flash_capacity,
            segment_size,
            min_reads,
            max_updates,
            min_age,
        })
    }
}

#[derive(Debug)]
struct GenerateOptions {
    output: PathBuf,
    requests: u64,
    keys: u64,
}

impl GenerateOptions {
    fn parse(args: &[String]) -> Result<Self, Box<dyn Error>> {
        let mut parser = ArgParser::new(args);
        let output = PathBuf::from(parser.required("--output")?);
        let requests = parser.optional_u64("--requests")?.unwrap_or(10_000);
        let keys = parser.optional_u64("--keys")?.unwrap_or(1_000);
        parser.finish()?;

        Ok(Self {
            output,
            requests,
            keys,
        })
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
  cargo run -- generate-trace --output <path> [--requests <n>] [--keys <n>]

Options:
  --dram-capacity <bytes>   DRAM cache capacity, default 1048576
  --flash-capacity <bytes>  Flash cache capacity, default 10485760
  --segment-size <bytes>    Simulated flash segment size, default 1048576
  --min-reads <n>           Flashield-lite admission threshold, default 2
  --max-updates <n>         Flashield-lite admission threshold, default 1
  --min-age <ticks>         Flashield-lite admission threshold, default 2"
}
