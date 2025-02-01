function __fish_noxe_complete
    # 检查当前命令行是否以 "noxe preview" 开头
    if not string match -q "noxe preview*" -- (commandline -p)
        return
    end

    # 获取补全项
    set -l completions (noxe list -ut -N 18446744073709551615)

    # 输出补全项
    printf "%s\n" $completions | sort -u
end

complete -c noxe -a "(__fish_noxe_complete)" -d "Note name"
complete -c noxe  -a "(__fish_complete_path)"
