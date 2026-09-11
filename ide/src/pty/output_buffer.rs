use std::collections::VecDeque;

use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine;
use schemars::JsonSchema;
use serde::Serialize;

pub const MAX_OUTPUT_BUFFER_BYTES: usize = 2 * 1024 * 1024;

#[derive(Clone, Debug, Eq, JsonSchema, PartialEq, Serialize)]
pub struct OutputFrame {
    pub sequence: u64,
    pub data: String,
}

#[derive(Clone, Debug, Eq, JsonSchema, PartialEq, Serialize)]
pub struct Replay {
    pub frames: Vec<OutputFrame>,
    pub truncated: bool,
    pub next_sequence: u64,
}

#[derive(Debug)]
struct BufferedFrame {
    frame: OutputFrame,
    byte_len: usize,
}

#[derive(Debug)]
pub struct OutputBuffer {
    capacity_bytes: usize,
    frames: VecDeque<BufferedFrame>,
    total_bytes: usize,
    next_sequence: u64,
    dropped_through_sequence: u64,
}

impl OutputBuffer {
    pub fn new(capacity_bytes: usize) -> Self {
        Self {
            capacity_bytes: capacity_bytes.min(MAX_OUTPUT_BUFFER_BYTES),
            frames: VecDeque::new(),
            total_bytes: 0,
            next_sequence: 1,
            dropped_through_sequence: 0,
        }
    }

    pub fn push(&mut self, bytes: Vec<u8>) -> OutputFrame {
        let frame = OutputFrame {
            sequence: self.next_sequence,
            data: BASE64_STANDARD.encode(&bytes),
        };
        self.next_sequence = self.next_sequence.saturating_add(1);
        if bytes.len() > self.capacity_bytes {
            self.dropped_through_sequence = self.dropped_through_sequence.max(frame.sequence);
            return frame;
        }

        self.total_bytes = self.total_bytes.saturating_add(bytes.len());
        self.frames.push_back(BufferedFrame {
            frame: frame.clone(),
            byte_len: bytes.len(),
        });

        while self.total_bytes > self.capacity_bytes && self.frames.len() > 1 {
            let evicted = self.frames.pop_front().expect("buffer contains a frame");
            self.total_bytes = self.total_bytes.saturating_sub(evicted.byte_len);
            self.dropped_through_sequence =
                self.dropped_through_sequence.max(evicted.frame.sequence);
        }

        frame
    }

    pub fn frames_after(&self, sequence: u64) -> Replay {
        let truncated = sequence < self.dropped_through_sequence;
        let frames = self
            .frames
            .iter()
            .filter(|frame| frame.frame.sequence > sequence)
            .map(|frame| frame.frame.clone())
            .collect();

        Replay {
            frames,
            truncated,
            next_sequence: self.next_sequence,
        }
    }

    /// What the buffer holds right now, for `shell::pty::sessions`.
    pub fn stats(&self) -> BufferStats {
        BufferStats {
            sequence: self.next_sequence.saturating_sub(1),
            frames: self.frames.len(),
            frame_bytes: self.total_bytes,
            truncated: self.dropped_through_sequence > 0,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BufferStats {
    /// Sequence of the last frame produced (0 before the first one).
    pub sequence: u64,
    pub frames: usize,
    pub frame_bytes: usize,
    pub truncated: bool,
}

#[cfg(test)]
mod tests {
    use super::OutputBuffer;

    #[test]
    fn evicts_old_frames_and_reports_truncation() {
        let mut buffer = OutputBuffer::new(8);
        buffer.push(vec![1, 2, 3, 4, 5]);
        let latest = buffer.push(vec![6, 7, 8, 9, 10]);

        let replay = buffer.frames_after(0);

        assert!(replay.truncated);
        assert_eq!(replay.frames, vec![latest.clone()]);
        assert_eq!(replay.next_sequence, latest.sequence + 1);
    }

    #[test]
    fn omits_an_oversized_frame_from_replay() {
        let mut buffer = OutputBuffer::new(8);
        let frame = buffer.push(vec![1; 9]);

        let replay = buffer.frames_after(0);

        assert!(replay.truncated);
        assert!(replay.frames.is_empty());
        assert_eq!(replay.next_sequence, frame.sequence + 1);
        assert!(buffer.total_bytes <= buffer.capacity_bytes);
    }
}
