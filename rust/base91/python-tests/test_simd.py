# SPDX-FileCopyrightText: 2026 Frederic Ruget <fred@atlant.is> (GitHub: @douzebis)
#
# SPDX-License-Identifier: MIT

"""The SIMD variant in the pybase91 extension (spec 0014 §2)."""

import random

import pytest

import pybase91

SIZES = (0, 1, 2, 12, 13, 25, 26, 27, 100, 1000, 10_000)
WIDTHS = (0, 32, 64, 96)


def _data(n: int) -> bytes:
    return random.Random(0x0014_0002 + n).randbytes(n)


def test_round_trip() -> None:
    for n in SIZES:
        data = _data(n)
        for wrap in WIDTHS:
            encoded = pybase91.simd_encode(data, wrap)
            if n:
                assert encoded.startswith(b"-")
            assert b"'" not in encoded
            assert pybase91.simd_decode(encoded) == data, (n, wrap)


def test_default_width_is_unwrapped() -> None:
    data = _data(1000)
    assert pybase91.simd_encode(data) == pybase91.simd_encode(data, 0)
    assert b"\n" not in pybase91.simd_encode(data)


def test_simd_decode_detects_henke_format() -> None:
    for n in SIZES:
        data = _data(n)
        assert pybase91.simd_decode(pybase91.encode(data)) == data


def _encode_in_chunks(data: bytes, size: int, wrap: int) -> bytes:
    encoder = pybase91.SimdEncoder(wrap)
    for i in range(0, len(data), size):
        encoder.update(data[i : i + size])
    return encoder.finish()


def _decode_in_chunks(encoded: bytes, size: int) -> bytes:
    decoder = pybase91.SimdDecoder()
    for i in range(0, len(encoded), size):
        decoder.update(encoded[i : i + size])
    return decoder.finish()


def test_streaming_matches_one_shot() -> None:
    for n in (0, 1, 13, 100, 1000):
        data = _data(n)
        for wrap in (0, 64):
            encoded = pybase91.simd_encode(data, wrap)
            for size in (1, 2, 7, 16, 4096):
                assert _encode_in_chunks(data, size, wrap) == encoded, (n, wrap, size)
                assert _decode_in_chunks(encoded, size) == data, (n, wrap, size)


def test_streaming_decoder_detects_henke_format() -> None:
    data = _data(1000)
    assert _decode_in_chunks(pybase91.encode(data), 7) == data


def test_finish_resets() -> None:
    encoder = pybase91.SimdEncoder(64)
    decoder = pybase91.SimdDecoder()
    for n in (100, 1000):
        data = _data(n)
        encoder.update(data)
        encoded = encoder.finish()
        assert encoded == pybase91.simd_encode(data, 64)
        decoder.update(encoded)
        assert decoder.finish() == data


def test_invalid_width() -> None:
    for call in (
        lambda: pybase91.simd_encode(b"data", 5),
        lambda: pybase91.SimdEncoder(33),
    ):
        with pytest.raises(pybase91.Error) as info:
            call()
        assert isinstance(info.value, ValueError)
        assert info.value.kind == "invalid_width"
        assert info.value.offset is None


def _corrupted() -> bytes:
    encoded = pybase91.simd_encode(_data(1000))
    # `'` is outside the SIMD alphabet.
    return encoded[:10] + b"'" + encoded[11:]


def test_invalid_character_has_offset() -> None:
    with pytest.raises(pybase91.Error) as info:
        pybase91.simd_decode(_corrupted())
    assert info.value.kind == "invalid_character"
    assert info.value.offset == 10


def test_streaming_decoder_error_persists() -> None:
    decoder = pybase91.SimdDecoder()
    with pytest.raises(pybase91.Error):
        decoder.update(_corrupted())
    with pytest.raises(pybase91.Error):
        decoder.update(b"#$")
    with pytest.raises(pybase91.Error):
        decoder.finish()


def test_simd_level() -> None:
    assert pybase91.simd_level() in {"avx2", "sse4.1", "neon", "scalar"}
