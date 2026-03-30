complete -c base91 -s o -l output -d 'Write output to FILE instead of standard output.' -r -F
complete -c base91 -s m -l buffer -d 'Use SIZE bytes of memory for I/O buffers (suffixes: b, K, M). Default: 64K.' -r
complete -c base91 -s w -l wrap -d 'Wrap encoded lines after COLS characters (0 = no wrap; default 76). With --simd, COLS must be a multiple of 32. Has no effect when decoding.' -r
complete -c base91 -s d -l decode -d 'Decode data instead of encoding.'
complete -c base91 -s v -l verbose -d 'Print statistics to stderr. Repeat (-vv) for extra verbosity.'
complete -c base91 -l simd -d 'Use the SIMD fixed-width variant for encoding. Output begins with \'-\' and uses the SIMD alphabet (0x23-0x26, 0x28-0x7E); not compatible with legacy Henke decoders. Ignored when decoding.'
complete -c base91 -s h -l help -d 'Print help (see more with \'--help\')'
complete -c base91 -s V -l version -d 'Print version'
