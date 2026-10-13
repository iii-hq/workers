package main

import (
	"encoding/base64"
	"fmt"
	"strings"
	"testing"
)

func BenchmarkDecodeCloudDocumentRequest(b *testing.B) {
	for _, size := range []int{1 << 20, 8 << 20, 16 << 20} {
		b.Run(fmt.Sprintf("fileMiB=%d", size>>20), func(b *testing.B) {
			// Measure only the worker's map-to-typed-request boundary. Transport
			// parsing, base64 decoding, multipart framing, and Meta are excluded.
			input := map[string]any{
				"file_name": "synthetic.txt", "mimetype": "text/plain",
				"data_base64": base64.StdEncoding.EncodeToString([]byte(strings.Repeat("x", size))),
			}
			b.SetBytes(int64(size))
			b.ReportAllocs()
			for b.Loop() {
				var request CloudDocumentUploadRequest
				if err := decodeCloudRequest(input, &request); err != nil {
					b.Fatal(err)
				}
			}
		})
	}
}
