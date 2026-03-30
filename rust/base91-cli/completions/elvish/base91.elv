
use builtin;
use str;

set edit:completion:arg-completer[base91] = {|@words|
    fn spaces {|n|
        builtin:repeat $n ' ' | str:join ''
    }
    fn cand {|text desc|
        edit:complex-candidate $text &display=$text' '(spaces (- 14 (wcswidth $text)))$desc
    }
    var command = 'base91'
    for word $words[1..-1] {
        if (str:has-prefix $word '-') {
            break
        }
        set command = $command';'$word
    }
    var completions = [
        &'base91'= {
            cand -o 'Write output to FILE instead of standard output.'
            cand --output 'Write output to FILE instead of standard output.'
            cand -m 'Use SIZE bytes of memory for I/O buffers (suffixes: b, K, M). Default: 64K.'
            cand --buffer 'Use SIZE bytes of memory for I/O buffers (suffixes: b, K, M). Default: 64K.'
            cand -w 'Wrap encoded lines after COLS characters (0 = no wrap; default 76). With --simd, COLS must be a multiple of 32. Has no effect when decoding.'
            cand --wrap 'Wrap encoded lines after COLS characters (0 = no wrap; default 76). With --simd, COLS must be a multiple of 32. Has no effect when decoding.'
            cand -d 'Decode data instead of encoding.'
            cand --decode 'Decode data instead of encoding.'
            cand -v 'Print statistics to stderr. Repeat (-vv) for extra verbosity.'
            cand --verbose 'Print statistics to stderr. Repeat (-vv) for extra verbosity.'
            cand --simd 'Use the SIMD fixed-width variant for encoding. Output begins with ''-'' and uses the SIMD alphabet (0x23-0x26, 0x28-0x7E); not compatible with legacy Henke decoders. Ignored when decoding.'
            cand -h 'Print help (see more with ''--help'')'
            cand --help 'Print help (see more with ''--help'')'
            cand -V 'Print version'
            cand --version 'Print version'
        }
    ]
    $completions[$command]
}
