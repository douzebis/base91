// SPDX-FileCopyrightText: 2026 Frederic Ruget <fred@atlant.is> (GitHub: @douzebis)
//
// SPDX-License-Identifier: MIT

package base91_test

import (
	"bytes"
	"crypto/sha256"
	"fmt"
	"os"
	"path/filepath"
	"runtime"
	"testing"

	"github.com/douzebis/base91/go"
)

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

func sha256hex(data []byte) string {
	sum := sha256.Sum256(data)
	return fmt.Sprintf("%x", sum)
}

// fixture reads a binary test fixture from the Rust crate's fixtures directory.
// The fixtures are shared between the Rust and Go test suites.
//
// The BASE91_FIXTURES_DIR environment variable overrides the default path,
// which is useful in Nix sandboxes where runtime.Caller(0) returns the
// compile-time source path rather than the runtime sandbox path.
func fixture(name string) []byte {
	var dir string
	if d := os.Getenv("BASE91_FIXTURES_DIR"); d != "" {
		dir = d
	} else {
		_, thisFile, _, _ := runtime.Caller(0)
		// go/base91_test.go → ../rust/base91/tests/fixtures/<name>
		dir = filepath.Join(filepath.Dir(thisFile), "..", "rust", "base91", "tests", "fixtures")
	}
	path := filepath.Join(dir, name)
	data, err := os.ReadFile(path)
	if err != nil {
		panic(fmt.Sprintf("fixture %q: %v", name, err))
	}
	return data
}

// ---------------------------------------------------------------------------
// Reference vector SHA-256 constants (computed from C reference output)
// ---------------------------------------------------------------------------

const (
	sha256Bit0B91 = "7a422b32e93f19ac3ce5404eef902ebc5d455ee3908372be39ef89d05f561885"
	sha256Bit1B91 = "f39d828129685524431b58c7cfb9db12b64d578fc7aa2b8f556aaa04f84aaeb0"
	sha256Rnd0B91 = "c1bc60717b1b59a9f78fdfb4224ba893f9e205cf614667380d8efd1d7db516df"
	sha256Rnd1B91 = "535df3e3603624890f0b16b5186c7e22b00a5cd33c38dc06fb4090b9346134fc"
	sha256Rnd1Dat = "dc5ce0f754128136c7ff8cebb79fbd0758f6e5007040ddce2c220021b5b59d2e"
)

// ---------------------------------------------------------------------------
// Basic round-trip tests
// ---------------------------------------------------------------------------

func TestRoundTripEmpty(t *testing.T) {
	enc := base91.Encode(nil)
	dec := base91.Decode(enc)
	if len(dec) != 0 {
		t.Fatalf("expected empty, got %d bytes", len(dec))
	}
}

func TestRoundTripHello(t *testing.T) {
	input := []byte("Hello, world!")
	dec := base91.Decode(base91.Encode(input))
	if !bytes.Equal(dec, input) {
		t.Fatalf("round-trip mismatch")
	}
}

func TestRoundTripAllBytes(t *testing.T) {
	input := make([]byte, 256)
	for i := range input {
		input[i] = byte(i)
	}
	dec := base91.Decode(base91.Encode(input))
	if !bytes.Equal(dec, input) {
		t.Fatalf("round-trip mismatch for all-bytes input")
	}
}

func TestRoundTripAllZeros(t *testing.T) {
	input := make([]byte, 1024)
	dec := base91.Decode(base91.Encode(input))
	if !bytes.Equal(dec, input) {
		t.Fatalf("round-trip mismatch for all-zeros input")
	}
}

// ---------------------------------------------------------------------------
// Size hint tests
// ---------------------------------------------------------------------------

func TestEncodeSizeHint(t *testing.T) {
	for length := 0; length <= 1024; length++ {
		input := make([]byte, length)
		for i := range input {
			input[i] = byte(i % 256)
		}
		enc := base91.Encode(input)
		hint := base91.EncodeSizeHint(length)
		if len(enc) > hint {
			t.Fatalf("EncodeSizeHint(%d)=%d < actual %d", length, hint, len(enc))
		}
	}
}

func TestDecodeSizeHint(t *testing.T) {
	for length := 0; length <= 1024; length++ {
		input := make([]byte, length)
		for i := range input {
			input[i] = byte(i % 256)
		}
		enc := base91.Encode(input)
		dec := base91.Decode(enc)
		hint := base91.DecodeSizeHint(len(enc))
		if len(dec) > hint {
			t.Fatalf("DecodeSizeHint(%d)=%d < actual %d", len(enc), hint, len(dec))
		}
	}
}

// ---------------------------------------------------------------------------
// Streaming encoder tests
// ---------------------------------------------------------------------------

func TestStreamingEncoderMatchesBulk(t *testing.T) {
	input := make([]byte, 1024)
	for i := range input {
		input[i] = byte(i % 256)
	}
	bulk := base91.Encode(input)

	// Feed in chunks of varying sizes
	for _, chunkSize := range []int{1, 3, 7, 13, 64, 256, 1024} {
		out := make([]byte, 0, base91.EncodeSizeHint(len(input)))
		var enc base91.Encoder
		for off := 0; off < len(input); off += chunkSize {
			end := off + chunkSize
			if end > len(input) {
				end = len(input)
			}
			enc.Write(input[off:end], &out)
		}
		enc.Finish(&out)
		if !bytes.Equal(out, bulk) {
			t.Fatalf("streaming chunk size %d: mismatch vs bulk", chunkSize)
		}
	}
}

func TestStreamingDecoderMatchesBulk(t *testing.T) {
	input := make([]byte, 1024)
	for i := range input {
		input[i] = byte(i % 256)
	}
	enc := base91.Encode(input)
	bulk := base91.Decode(enc)

	for _, chunkSize := range []int{1, 3, 7, 13, 64, 256, 1024} {
		out := make([]byte, 0, base91.DecodeSizeHint(len(enc)))
		dec := base91.Decoder{}
		for off := 0; off < len(enc); off += chunkSize {
			end := off + chunkSize
			if end > len(enc) {
				end = len(enc)
			}
			dec.Write(enc[off:end], &out)
		}
		dec.Finish(&out)
		if !bytes.Equal(out, bulk) {
			t.Fatalf("streaming chunk size %d: mismatch vs bulk", chunkSize)
		}
	}
}

// ---------------------------------------------------------------------------
// Non-alphabet filtering
// ---------------------------------------------------------------------------

func TestDecodeIgnoresNonAlphabet(t *testing.T) {
	input := []byte("Hello, world!")
	enc := base91.Encode(input)
	// Sprinkle spaces (non-alphabet) throughout
	noisy := make([]byte, 0, len(enc)*2)
	for _, b := range enc {
		noisy = append(noisy, b, ' ')
	}
	dec := base91.Decode(noisy)
	if !bytes.Equal(dec, input) {
		t.Fatalf("non-alphabet filtering failed")
	}
}

// ---------------------------------------------------------------------------
// Reference vector tests (wire-compat with C reference)
// ---------------------------------------------------------------------------

func TestEncodeMatchesReferenceBit0(t *testing.T) {
	dat := fixture("bit0.dat")
	enc := base91.Encode(dat)
	if got := sha256hex(enc); got != sha256Bit0B91 {
		t.Fatalf("bit0 encode SHA-256 mismatch:\n got  %s\n want %s", got, sha256Bit0B91)
	}
}

func TestEncodeMatchesReferenceBit1(t *testing.T) {
	dat := fixture("bit1.dat")
	enc := base91.Encode(dat)
	if got := sha256hex(enc); got != sha256Bit1B91 {
		t.Fatalf("bit1 encode SHA-256 mismatch:\n got  %s\n want %s", got, sha256Bit1B91)
	}
}

func TestEncodeMatchesReferenceRnd0(t *testing.T) {
	dat := fixture("rnd0.dat")
	enc := base91.Encode(dat)
	if got := sha256hex(enc); got != sha256Rnd0B91 {
		t.Fatalf("rnd0 encode SHA-256 mismatch:\n got  %s\n want %s", got, sha256Rnd0B91)
	}
}

func TestEncodeMatchesReferenceRnd1(t *testing.T) {
	dat := fixture("rnd1.dat")
	enc := base91.Encode(dat)
	if got := sha256hex(enc); got != sha256Rnd1B91 {
		t.Fatalf("rnd1 encode SHA-256 mismatch:\n got  %s\n want %s", got, sha256Rnd1B91)
	}
}

func TestDecodeMatchesReference(t *testing.T) {
	// rnd0.dat is the base91 encoding of rnd1.dat
	rnd0 := fixture("rnd0.dat")
	dec := base91.Decode(rnd0)
	if got := sha256hex(dec); got != sha256Rnd1Dat {
		t.Fatalf("decode SHA-256 mismatch:\n got  %s\n want %s", got, sha256Rnd1Dat)
	}
}

// ---------------------------------------------------------------------------
// Round-trips on fixtures
// ---------------------------------------------------------------------------

func TestRoundTripFixtures(t *testing.T) {
	for _, name := range []string{"bit0.dat", "bit1.dat", "rnd0.dat", "rnd1.dat"} {
		t.Run(name, func(t *testing.T) {
			dat := fixture(name)
			if got := base91.Decode(base91.Encode(dat)); !bytes.Equal(got, dat) {
				t.Fatalf("round-trip mismatch for %s", name)
			}
		})
	}
}
