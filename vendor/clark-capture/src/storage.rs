use crate::{CaptureError, CaptureEvent};
use sha2::{Digest, Sha256};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

pub fn sanitize_segment(value: &str) -> String {
    let sanitized: String = value
        .trim()
        .chars()
        .map(|character| match character {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '.' | '_' | '-' => character,
            _ => '_',
        })
        .collect();
    if sanitized.is_empty() || sanitized.chars().all(|character| character == '.') {
        "unnamed".to_owned()
    } else {
        sanitized
    }
}
#[derive(Clone, Debug)]
pub struct FileEventStore {
    root: PathBuf,
    project: String,
}

impl FileEventStore {
    pub fn new(root: impl Into<PathBuf>, project: &str) -> Self {
        Self {
            root: root.into(),
            project: sanitize_segment(project),
        }
    }

    pub fn append(&self, event: &CaptureEvent) -> Result<PathBuf, CaptureError> {
        let date = event.timestamp.get(..10).unwrap_or("unknown-date");
        let directory = self.root.join(&self.project).join(date);
        fs::create_dir_all(&directory)?;
        let path = directory.join("events.ndjson");
        let mut bytes = serde_json::to_vec(event)?;
        bytes.push(b'\n');
        let mut file = OpenOptions::new().create(true).append(true).open(&path)?;
        file.write_all(&bytes)?;
        Ok(path)
    }

    pub fn attach(&self, source: &Path, timestamp: &str) -> Result<StoredAttachment, CaptureError> {
        let bytes = fs::read(source)?;
        let sha256 = hex::encode(Sha256::digest(&bytes));
        let date = timestamp.get(..10).unwrap_or("unknown-date");
        let directory = self.root.join(&self.project).join(date).join("attachments");
        fs::create_dir_all(&directory)?;
        let basename = source
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("attachment");
        let filename = format!("{}-{}", sha256, sanitize_segment(basename));
        let destination = directory.join(&filename);
        match OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&destination)
        {
            Ok(mut file) => file.write_all(&bytes)?,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.into()),
        }
        Ok(StoredAttachment {
            relative_path: PathBuf::from(&self.project)
                .join(date)
                .join("attachments")
                .join(filename),
            sha256,
            size: bytes.len() as u64,
        })
    }
}

#[derive(Clone, Debug)]
pub struct StoredAttachment {
    pub relative_path: PathBuf,
    pub sha256: String,
    pub size: u64,
}
