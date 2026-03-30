
using namespace System.Management.Automation
using namespace System.Management.Automation.Language

Register-ArgumentCompleter -Native -CommandName 'base91' -ScriptBlock {
    param($wordToComplete, $commandAst, $cursorPosition)

    $commandElements = $commandAst.CommandElements
    $command = @(
        'base91'
        for ($i = 1; $i -lt $commandElements.Count; $i++) {
            $element = $commandElements[$i]
            if ($element -isnot [StringConstantExpressionAst] -or
                $element.StringConstantType -ne [StringConstantType]::BareWord -or
                $element.Value.StartsWith('-') -or
                $element.Value -eq $wordToComplete) {
                break
        }
        $element.Value
    }) -join ';'

    $completions = @(switch ($command) {
        'base91' {
            [CompletionResult]::new('-o', '-o', [CompletionResultType]::ParameterName, 'Write output to FILE instead of standard output.')
            [CompletionResult]::new('--output', '--output', [CompletionResultType]::ParameterName, 'Write output to FILE instead of standard output.')
            [CompletionResult]::new('-m', '-m', [CompletionResultType]::ParameterName, 'Use SIZE bytes of memory for I/O buffers (suffixes: b, K, M). Default: 64K.')
            [CompletionResult]::new('--buffer', '--buffer', [CompletionResultType]::ParameterName, 'Use SIZE bytes of memory for I/O buffers (suffixes: b, K, M). Default: 64K.')
            [CompletionResult]::new('-w', '-w', [CompletionResultType]::ParameterName, 'Wrap encoded lines after COLS characters (0 = no wrap; default 76). With --simd, COLS must be a multiple of 32. Has no effect when decoding.')
            [CompletionResult]::new('--wrap', '--wrap', [CompletionResultType]::ParameterName, 'Wrap encoded lines after COLS characters (0 = no wrap; default 76). With --simd, COLS must be a multiple of 32. Has no effect when decoding.')
            [CompletionResult]::new('-d', '-d', [CompletionResultType]::ParameterName, 'Decode data instead of encoding.')
            [CompletionResult]::new('--decode', '--decode', [CompletionResultType]::ParameterName, 'Decode data instead of encoding.')
            [CompletionResult]::new('-v', '-v', [CompletionResultType]::ParameterName, 'Print statistics to stderr. Repeat (-vv) for extra verbosity.')
            [CompletionResult]::new('--verbose', '--verbose', [CompletionResultType]::ParameterName, 'Print statistics to stderr. Repeat (-vv) for extra verbosity.')
            [CompletionResult]::new('--simd', '--simd', [CompletionResultType]::ParameterName, 'Use the SIMD fixed-width variant for encoding. Output begins with ''-'' and uses the SIMD alphabet (0x23-0x26, 0x28-0x7E); not compatible with legacy Henke decoders. Ignored when decoding.')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            [CompletionResult]::new('-V', '-V ', [CompletionResultType]::ParameterName, 'Print version')
            [CompletionResult]::new('--version', '--version', [CompletionResultType]::ParameterName, 'Print version')
            break
        }
    })

    $completions.Where{ $_.CompletionText -like "$wordToComplete*" } |
        Sort-Object -Property ListItemText
}
