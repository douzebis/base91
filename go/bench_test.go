// SPDX-FileCopyrightText: 2026 Frederic Ruget <fred@atlant.is> (GitHub: @douzebis)
//
// SPDX-License-Identifier: MIT

package base91_test

import (
	"math/rand"
	"testing"

	"github.com/douzebis/base91/go"
)

const benchSize = 1024 * 1024 // 1 MiB

func randomInput(size int) []byte {
	r := rand.New(rand.NewSource(42))
	buf := make([]byte, size)
	r.Read(buf)
	return buf
}

func BenchmarkEncode(b *testing.B) {
	input := randomInput(benchSize)
	out := make([]byte, 0, base91.EncodeSizeHint(len(input)))
	b.SetBytes(int64(len(input)))
	b.ResetTimer()
	for i := 0; i < b.N; i++ {
		out = out[:0]
		var enc base91.Encoder
		enc.Write(input, &out)
		enc.Finish(&out)
	}
}

func BenchmarkDecode(b *testing.B) {
	encoded := base91.Encode(randomInput(benchSize))
	out := make([]byte, 0, base91.DecodeSizeHint(len(encoded)))
	b.SetBytes(int64(len(encoded)))
	b.ResetTimer()
	for i := 0; i < b.N; i++ {
		out = out[:0]
		var dec base91.Decoder
		dec.Write(encoded, &out)
		dec.Finish(&out)
	}
}
