// SPDX-FileCopyrightText: 2026 Frederic Ruget <fred@atlant.is> (GitHub: @douzebis)
//
// SPDX-License-Identifier: MIT

// Package base91 implements basE91 binary-to-text encoding.
//
// A pure-Go implementation of the basE91 algorithm invented by Joachim Henke.
// Wire-format compatible with the C reference implementation at
// http://base91.sourceforge.net/.
//
// # Quick start
//
//	encoded := base91.Encode([]byte("Hello, world!"))
//	decoded, _ := base91.Decode(encoded)
//	// decoded == []byte("Hello, world!")
//
// # Streaming
//
// For large or chunked inputs use [Encoder] and [Decoder] directly:
//
//	var enc base91.Encoder
//	enc.Write(chunk1)
//	enc.Write(chunk2)
//	encoded := enc.Finish()
package base91

// enctab maps a value 0-90 to its base91 ASCII character.
var enctab = [91]byte{
	'A', 'B', 'C', 'D', 'E', 'F', 'G', 'H', 'I', 'J', 'K', 'L', 'M',
	'N', 'O', 'P', 'Q', 'R', 'S', 'T', 'U', 'V', 'W', 'X', 'Y', 'Z',
	'a', 'b', 'c', 'd', 'e', 'f', 'g', 'h', 'i', 'j', 'k', 'l', 'm',
	'n', 'o', 'p', 'q', 'r', 's', 't', 'u', 'v', 'w', 'x', 'y', 'z',
	'0', '1', '2', '3', '4', '5', '6', '7', '8', '9', '!', '#', '$',
	'%', '&', '(', ')', '*', '+', ',', '.', '/', ':', ';', '<', '=',
	'>', '?', '@', '[', ']', '^', '_', '`', '{', '|', '}', '~', '"',
}

// dectab maps an ASCII byte to its base91 value (0-90), or 91 for non-alphabet.
var dectab = [256]byte{
	91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91,
	91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91,
	91, 62, 90, 63, 64, 65, 66, 91, 67, 68, 69, 70, 71, 91, 72, 73,
	52, 53, 54, 55, 56, 57, 58, 59, 60, 61, 74, 75, 76, 77, 78, 79,
	80, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14,
	15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 81, 91, 82, 83, 84,
	85, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40,
	41, 42, 43, 44, 45, 46, 47, 48, 49, 50, 51, 86, 87, 88, 89, 91,
	91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91,
	91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91,
	91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91,
	91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91,
	91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91,
	91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91,
	91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91,
	91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91,
}

// EncodeSizeHint returns a conservative upper bound on the encoded length for
// inputLen input bytes. Pre-allocating this capacity avoids reallocation.
func EncodeSizeHint(inputLen int) int {
	return (inputLen*16+12)/13 + 2
}

// DecodeSizeHint returns a conservative upper bound on the decoded length for
// inputLen encoded bytes. Pre-allocating this capacity avoids reallocation.
func DecodeSizeHint(inputLen int) int {
	return inputLen*7/8 + 1
}

// Encoder holds streaming encoder state. The zero value is ready to use.
type Encoder struct {
	queue uint32
	nbits uint32
}

// Write encodes src and appends the base91 characters to dst.
// Returns the number of bytes appended.
func (e *Encoder) Write(src []byte, dst *[]byte) int {
	n := 0
	queue := e.queue
	nbits := e.nbits
	for _, b := range src {
		queue |= uint32(b) << nbits
		nbits += 8
		if nbits > 13 {
			val := queue & 8191
			if val > 88 {
				queue >>= 13
				nbits -= 13
			} else {
				val = queue & 16383
				queue >>= 14
				nbits -= 14
			}
			*dst = append(*dst, enctab[val%91], enctab[val/91])
			n += 2
		}
	}
	e.queue = queue
	e.nbits = nbits
	return n
}

// Finish flushes any remaining bits and appends 0-2 bytes to dst.
// The encoder is reset to its zero state and may be reused.
func (e *Encoder) Finish(dst *[]byte) int {
	n := 0
	if e.nbits > 0 {
		*dst = append(*dst, enctab[e.queue%91])
		n++
		if e.nbits > 7 || e.queue > 90 {
			*dst = append(*dst, enctab[e.queue/91])
			n++
		}
	}
	e.queue = 0
	e.nbits = 0
	return n
}

// Decoder holds streaming decoder state. The zero value is ready to use.
type Decoder struct {
	queue  uint32
	nbits  uint32
	val    uint32 // pending first character value; valid only when hasVal is true
	hasVal bool
}

// Write decodes src (base91 characters) and appends decoded bytes to dst.
// Non-alphabet bytes are silently ignored.
// Returns the number of bytes appended.
func (d *Decoder) Write(src []byte, dst *[]byte) int {
	n := 0
	queue := d.queue
	nbits := d.nbits
	val := d.val
	hasVal := d.hasVal
	for _, b := range src {
		c := uint32(dectab[b])
		if c == 91 {
			continue
		}
		if !hasVal {
			val = c
			hasVal = true
		} else {
			v := val + c*91
			hasVal = false
			queue |= v << nbits
			if v&8191 > 88 {
				nbits += 13
			} else {
				nbits += 14
			}
			*dst = append(*dst, byte(queue))
			queue >>= 8
			nbits -= 8
			n++
			if nbits >= 8 {
				*dst = append(*dst, byte(queue))
				queue >>= 8
				nbits -= 8
				n++
			}
		}
	}
	d.queue = queue
	d.nbits = nbits
	d.val = val
	d.hasVal = hasVal
	return n
}

// Finish flushes any remaining partial value and appends 0-1 bytes to dst.
// The decoder is reset to its zero state and may be reused.
func (d *Decoder) Finish(dst *[]byte) int {
	n := 0
	if d.hasVal {
		*dst = append(*dst, byte(d.queue)|byte(d.val<<d.nbits))
		n++
	}
	d.queue = 0
	d.nbits = 0
	d.val = 0
	d.hasVal = false
	return n
}

// Encode encodes input to a new byte slice of base91 characters.
func Encode(input []byte) []byte {
	out := make([]byte, 0, EncodeSizeHint(len(input)))
	var enc Encoder
	enc.Write(input, &out)
	enc.Finish(&out)
	return out
}

// Decode decodes input (base91 characters) to a new byte slice.
// Non-alphabet bytes are silently ignored.
func Decode(input []byte) []byte {
	out := make([]byte, 0, DecodeSizeHint(len(input)))
	var dec Decoder
	dec.Write(input, &out)
	dec.Finish(&out)
	return out
}
