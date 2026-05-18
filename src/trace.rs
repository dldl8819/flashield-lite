use std::error::Error;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    Get,
    Set,
    Update,
    Delete,
}

impl Operation {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "get" => Ok(Self::Get),
            "set" => Ok(Self::Set),
            "update" => Ok(Self::Update),
            "delete" => Ok(Self::Delete),
            other => Err(format!("unsupported operation: {other}")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraceEvent {
    pub timestamp: u64,
    pub op: Operation,
    pub key: String,
    pub size: u64,
}

pub fn read_trace(path: &Path) -> Result<Vec<TraceEvent>, Box<dyn Error>> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let mut lines = reader.lines();

    let Some(header) = lines.next() else {
        return Err("trace is empty".into());
    };

    if header?.trim() != "timestamp,op,key,size" {
        return Err("trace header must be exactly: timestamp,op,key,size".into());
    }

    let mut events = Vec::new();
    for (line_index, line) in lines.enumerate() {
        let line_number = line_index + 2;
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        events.push(parse_trace_line(&line).map_err(|err| format!("line {line_number}: {err}"))?);
    }
    Ok(events)
}

pub fn parse_trace_line(line: &str) -> Result<TraceEvent, String> {
    let fields: Vec<&str> = line.split(',').map(str::trim).collect();
    if fields.len() != 4 {
        return Err("expected four comma-separated fields".to_string());
    }

    let timestamp = fields[0]
        .parse::<u64>()
        .map_err(|_| "timestamp must be an unsigned integer".to_string())?;
    let op = Operation::parse(fields[1])?;
    let key = fields[2];
    if key.is_empty() {
        return Err("key must not be empty".to_string());
    }
    let size = fields[3]
        .parse::<u64>()
        .map_err(|_| "size must be an unsigned integer".to_string())?;

    Ok(TraceEvent {
        timestamp,
        op,
        key: key.to_string(),
        size,
    })
}

#[cfg(test)]
mod tests {
    use super::{parse_trace_line, Operation, TraceEvent};

    #[test]
    fn parses_valid_trace_line() {
        let event = parse_trace_line("42,set,synthetic:1,128").unwrap();

        assert_eq!(
            event,
            TraceEvent {
                timestamp: 42,
                op: Operation::Set,
                key: "synthetic:1".to_string(),
                size: 128,
            }
        );
    }

    #[test]
    fn rejects_unknown_operation() {
        let err = parse_trace_line("1,merge,synthetic:1,128").unwrap_err();
        assert!(err.contains("unsupported operation"));
    }
}
