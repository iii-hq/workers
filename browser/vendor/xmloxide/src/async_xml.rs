//! Async XML parsing via `tokio::io::AsyncRead`.
//!
//! This module provides [`parse_async`], which reads from any `AsyncRead`
//! source and builds a [`Document`] using the push parser internally.
//!
//! Requires the `async` feature.
//!
//! # Examples
//!
//! ```no_run
//! # #[cfg(feature = "async")]
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! use xmloxide::async_xml::parse_async;
//!
//! let file = tokio::fs::File::open("data.xml").await?;
//! let doc = parse_async(file).await?;
//! let root = doc.root_element().unwrap();
//! println!("Root: {:?}", doc.node_name(root));
//! # Ok(())
//! # }
//! ```

use tokio::io::{AsyncRead, AsyncReadExt};

use crate::error::ParseError;
use crate::parser::{ParseOptions, PushParser};
use crate::tree::Document;

/// Default buffer size for async reads (8 KiB).
const DEFAULT_BUF_SIZE: usize = 8192;

/// Parses XML from an `AsyncRead` source using default options.
///
/// Reads the source in chunks and feeds them to the push parser.
///
/// # Errors
///
/// Returns a `ParseError` if the XML is malformed.
pub async fn parse_async<R: AsyncRead + Unpin>(reader: R) -> Result<Document, ParseError> {
    parse_async_with_options(reader, ParseOptions::default()).await
}

/// Parses XML from an `AsyncRead` source with the given parse options.
///
/// # Errors
///
/// Returns a `ParseError` if the XML is malformed.
pub async fn parse_async_with_options<R: AsyncRead + Unpin>(
    mut reader: R,
    options: ParseOptions,
) -> Result<Document, ParseError> {
    let mut parser = PushParser::with_options(options);
    let mut buf = vec![0u8; DEFAULT_BUF_SIZE];

    loop {
        let n = reader.read(&mut buf).await.map_err(|e| ParseError {
            message: format!("I/O error: {e}"),
            location: crate::error::SourceLocation {
                line: 0,
                column: 0,
                byte_offset: 0,
            },
            diagnostics: vec![],
        })?;
        if n == 0 {
            break;
        }
        parser.push(&buf[..n]);
    }

    parser.finish()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_parse_async_from_bytes() {
        let data = b"<root><child>Hello</child></root>";
        let cursor = std::io::Cursor::new(data);
        let doc = parse_async(cursor).await.unwrap();
        let root = doc.root_element().unwrap();
        assert_eq!(doc.node_name(root), Some("root"));
        assert_eq!(doc.text_content(root), "Hello");
    }

    #[tokio::test]
    async fn test_parse_async_empty_document() {
        let data = b"<root/>";
        let cursor = std::io::Cursor::new(data);
        let doc = parse_async(cursor).await.unwrap();
        let root = doc.root_element().unwrap();
        assert_eq!(doc.node_name(root), Some("root"));
    }

    #[tokio::test]
    async fn test_parse_async_with_options() {
        let data = b"<root><child>text</child></root>";
        let cursor = std::io::Cursor::new(data);
        let opts = ParseOptions::default().recover(true);
        let doc = parse_async_with_options(cursor, opts).await.unwrap();
        let root = doc.root_element().unwrap();
        assert_eq!(doc.node_name(root), Some("root"));
    }

    #[tokio::test]
    async fn test_parse_async_malformed() {
        let data = b"<root><</root>";
        let cursor = std::io::Cursor::new(data);
        let result = parse_async(cursor).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_parse_async_small_reads() {
        // Simulate a reader that yields one byte at a time
        let data = b"<root>Hello</root>";
        let cursor = SlowReader { data, pos: 0 };
        let doc = parse_async(cursor).await.unwrap();
        let root = doc.root_element().unwrap();
        assert_eq!(doc.text_content(root), "Hello");
    }

    /// Test reader that yields one byte at a time.
    struct SlowReader {
        data: &'static [u8],
        pos: usize,
    }

    impl AsyncRead for SlowReader {
        fn poll_read(
            mut self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
            buf: &mut tokio::io::ReadBuf<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            if self.pos >= self.data.len() {
                return std::task::Poll::Ready(Ok(()));
            }
            buf.put_slice(&self.data[self.pos..=self.pos]);
            self.pos += 1;
            std::task::Poll::Ready(Ok(()))
        }
    }
}
