# SPDX-FileCopyrightText: 2026 Frederic Ruget <fred@atlant.is> (GitHub: @douzebis)
#
# SPDX-License-Identifier: MIT

"""Streaming decoder of the pybase91 extension (spec 0006 §6, bug 5)."""

import random

import pybase91


def _data(n: int) -> bytes:
    return random.Random(0x0006_0005).randbytes(n)


def _decode_in_chunks(encoded: bytes, size: int) -> bytes:
    decoder = pybase91.Decoder()
    for i in range(0, len(encoded), size):
        decoder.update(encoded[i : i + size])
    return decoder.finish()


def test_decoder_one_byte_chunks() -> None:
    # On 0.2.2 this raised PanicException: a character pending from the
    # previous update() completed a pair the reservation did not cover.
    data = _data(20_000)
    encoded = pybase91.encode(data)
    assert _decode_in_chunks(encoded, 1) == data


def test_decoder_chunk_sizes_match_one_shot() -> None:
    for n in (0, 1, 2, 13, 100, 1000):
        data = _data(n)
        encoded = pybase91.encode(data)
        assert pybase91.decode(encoded) == data
        for size in (1, 2, 3, 5, 7, 16, 4096):
            assert _decode_in_chunks(encoded, size) == data, (n, size)
