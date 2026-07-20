use serde_json::Value;

#[derive(Clone, Debug, PartialEq)]
pub enum SseFrame {
    Json {
        event: Option<String>,
        value: Value,
    },
    Done,
    Malformed {
        event: Option<String>,
        preview: String,
    },
}

/// Incremental SSE decoder shared by provider adapters. It accepts arbitrary
/// transport fragmentation, supports data-only compatibility streams, bounds
/// buffered event data, and makes malformed terminal frames observable.
#[derive(Debug)]
pub struct SseDecoder {
    line_buffer: Vec<u8>,
    data: String,
    event: Option<String>,
    max_event_bytes: usize,
}

impl Default for SseDecoder {
    fn default() -> Self {
        Self::new(1024 * 1024)
    }
}

impl SseDecoder {
    pub fn new(max_event_bytes: usize) -> Self {
        Self {
            line_buffer: Vec::new(),
            data: String::new(),
            event: None,
            max_event_bytes: max_event_bytes.max(1),
        }
    }

    pub fn push(&mut self, chunk: &[u8]) -> Vec<SseFrame> {
        self.line_buffer.extend_from_slice(chunk);
        let mut frames = Vec::new();
        while let Some(newline) = self.line_buffer.iter().position(|byte| *byte == b'\n') {
            let mut line = self.line_buffer.drain(..=newline).collect::<Vec<_>>();
            line.pop();
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            let line = String::from_utf8_lossy(&line);
            self.consume_line(line.trim_end(), &mut frames);
        }
        frames
    }

    pub fn finish(&mut self) -> Vec<SseFrame> {
        let mut frames = Vec::new();
        if !self.line_buffer.is_empty() {
            let line = String::from_utf8_lossy(&self.line_buffer).into_owned();
            self.line_buffer.clear();
            self.consume_line(line.trim_end_matches('\r'), &mut frames);
        }
        self.flush_data(&mut frames);
        frames
    }

    fn consume_line(&mut self, line: &str, frames: &mut Vec<SseFrame>) {
        if line.is_empty() {
            self.flush_data(frames);
            self.event = None;
            return;
        }
        if line.starts_with(':') {
            return;
        }
        if let Some(event) = line.strip_prefix("event:") {
            self.event = Some(event.trim().to_string());
            return;
        }
        let Some(data) = line.strip_prefix("data:") else {
            return;
        };
        let data = data.strip_prefix(' ').unwrap_or(data);
        if data == "[DONE]" {
            self.data.clear();
            frames.push(SseFrame::Done);
            return;
        }
        if self.data.len().saturating_add(data.len()) > self.max_event_bytes {
            frames.push(SseFrame::Malformed {
                event: self.event.clone(),
                preview: "provider SSE event exceeded the configured size limit".into(),
            });
            self.data.clear();
            return;
        }
        self.data.push_str(data);
        // Compatibility endpoints sometimes omit blank event boundaries. Emit
        // as soon as the accumulated data is complete JSON.
        if serde_json::from_str::<Value>(&self.data).is_ok() {
            self.flush_data(frames);
        }
    }

    fn flush_data(&mut self, frames: &mut Vec<SseFrame>) {
        if self.data.is_empty() {
            return;
        }
        let data = std::mem::take(&mut self.data);
        match serde_json::from_str::<Value>(&data) {
            Ok(value) => frames.push(SseFrame::Json {
                event: self.event.clone(),
                value,
            }),
            Err(_) => frames.push(SseFrame::Malformed {
                event: self.event.clone(),
                preview: data.chars().take(160).collect(),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handles_fragmented_data_multiline_data_only_and_done() {
        let mut decoder = SseDecoder::default();
        assert!(decoder.push(b"event: message\nda").is_empty());
        let frames = decoder.push(b"ta: {\"x\":1}\n\ndata:{\"y\":2}\ndata: [DONE]\n\n");
        assert!(
            matches!(&frames[0], SseFrame::Json { event: Some(event), value } if event == "message" && value["x"] == 1)
        );
        assert!(matches!(&frames[1], SseFrame::Json { value, .. } if value["y"] == 2));
        assert!(matches!(&frames[2], SseFrame::Done));
    }

    #[test]
    fn reports_malformed_final_frame_and_enforces_bound() {
        let mut decoder = SseDecoder::new(8);
        let frames = decoder.push(b"data: {bad}\n\n");
        assert!(
            matches!(&frames[0], SseFrame::Malformed { preview, .. } if preview.contains("bad"))
        );
        let frames = decoder.push(b"data: 123456789\n");
        assert!(
            matches!(&frames[0], SseFrame::Malformed { preview, .. } if preview.contains("size limit"))
        );
    }
}
