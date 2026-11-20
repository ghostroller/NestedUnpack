use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::PathBuf;

use crate::i18n::{EngineKey, Message};

use super::open_file;

// Restrict ZIP's seekable view to the appended archive. The optional mask marks
// legacy EOCD counts when a complete ZIP64 directory is present.
pub(super) struct ZipView {
    file: File,
    start: u64,
    size: u64,
    position: u64,
    count_mask: Option<u64>,
}

impl ZipView {
    pub(super) fn new(
        mut file: File,
        start: u64,
        size: u64,
        count_mask: Option<u64>,
    ) -> io::Result<Self> {
        file.seek(SeekFrom::Start(start))?;
        Ok(Self {
            file,
            start,
            size,
            position: 0,
            count_mask,
        })
    }
}

impl Read for ZipView {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        let capacity = self
            .size
            .saturating_sub(self.position)
            .min(output.len() as u64) as usize;
        let read = self.file.read(&mut output[..capacity])?;
        let end = self.position + read as u64;
        if let Some(mask) = self.count_mask {
            let start = self.position.max(mask);
            let stop = end.min(mask + 4);
            if start < stop {
                output[(start - self.position) as usize..(stop - self.position) as usize]
                    .fill(0xff);
            }
        }
        self.position = end;
        Ok(read)
    }
}

impl Seek for ZipView {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        let position = seek_position(from, self.position, self.size)?;
        // ZIP scans for the central directory and may seek to the same offset
        // repeatedly while reading metadata; avoid redundant file seeks.
        if position != self.position {
            let offset = self.start.checked_add(position).ok_or_else(invalid_seek)?;
            self.file.seek(SeekFrom::Start(offset))?;
            self.position = position;
        }
        Ok(position)
    }
}

struct Part {
    file: File,
    start: u64,
    end: u64,
    position: u64,
}

pub(super) struct JoinedVolumes {
    parts: Vec<Part>,
    size: u64,
    position: u64,
}

impl JoinedVolumes {
    pub(super) fn open(paths: &[PathBuf]) -> io::Result<Self> {
        let mut size = 0u64;
        let mut parts = Vec::with_capacity(paths.len());
        for path in paths {
            let (file, length) = open_file(path)?;
            let start = size;
            size = size.checked_add(length).ok_or_else(|| {
                Message::from(EngineKey::SplitArchiveTooLarge).io(io::ErrorKind::InvalidData)
            })?;
            parts.push(Part {
                file,
                start,
                end: size,
                position: 0,
            });
        }
        Ok(Self {
            parts,
            size,
            position: 0,
        })
    }
}

impl Read for JoinedVolumes {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if output.is_empty() || self.position >= self.size {
            return Ok(0);
        }
        let index = self.parts.partition_point(|part| part.end <= self.position);
        let part = &mut self.parts[index];
        let position = self.position - part.start;
        if position != part.position {
            part.file.seek(SeekFrom::Start(position))?;
            part.position = position;
        }
        let capacity = (part.end - self.position).min(output.len() as u64) as usize;
        let read = part.file.read(&mut output[..capacity])?;
        if read == 0 {
            return Err(
                Message::from(EngineKey::TruncatedSplitArchive).io(io::ErrorKind::UnexpectedEof)
            );
        }
        part.position += read as u64;
        self.position += read as u64;
        Ok(read)
    }
}

impl Seek for JoinedVolumes {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        self.position = seek_position(from, self.position, self.size)?;
        Ok(self.position)
    }
}

fn seek_position(from: SeekFrom, position: u64, size: u64) -> io::Result<u64> {
    match from {
        SeekFrom::Start(offset) => Ok(offset),
        SeekFrom::Current(offset) => position.checked_add_signed(offset).ok_or_else(invalid_seek),
        SeekFrom::End(offset) => size.checked_add_signed(offset).ok_or_else(invalid_seek),
    }
}

fn invalid_seek() -> io::Error {
    Message::from(EngineKey::InvalidArchiveSeek).io(io::ErrorKind::InvalidInput)
}
