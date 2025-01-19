一个用于创建笔记和预览笔记的工具。笔记分为文件和文件夹两种形式，笔记类型分为markdown和typst两种类型。

# 1. 依赖

该程序依赖于[tinymist](https://github.com/Myriad-Dreamin/tinymist)(预览typst)和[glow](https://github.com/charmbracelet/glow)(预览markdown)，请确保这两个程序已经安装。

# 2. 安装
  
```shell
cargo install noxe
```

# 3. 使用

```shell
noxe new myNote
noxe new myFileNote.md
noxe preview myNote # 在$NOXE_DIR下查找并预览myNote, $NOXE_DIR默认为当前目录
noxe preview ./myNote # 查看当前目录下的myNote
```

# 4. 笔记的目录结构

文件夹形式的笔记，笔记的默认目录结构如下：

```
myResearchNote/
├── bibliography/
│   ├── refs.bib
├── chapter/
├── images/
└── main.typ
```

用户可以通过yaml文件配置笔记的目录结构并通过`--note_template`指定配置文件的路径。配置示例如下：

```yaml
paths:
  images: {} # 空文件夹
  bibliography:
    refs.bib: |
      % @article{netwok2020,
      %   title={At-scale impact of the {Net Wok}: A culinarily holistic investigation of distributed dumplings},
      %   author={Astley, Rick and Morris, Linda},
      %   journal={Armenian Journal of Proceedings},
      %   volume={61},
      %   pages={192--219},
      %   year=2020,
      %   publisher={Automatic Publishing Inc.}
      % }
  chapter: {}

# 若文件类型为typst则向主文件中插入内容（不论是文件夹还是文件形式的笔记）
main.typ: |
  #import "@local/common:0.0.1": *
  #show: common.with()

# 若文件类型为markdown则向主文件中插入内容
main.md: |
  # My Research Note
  This is my research note.

```

# 5. TODO

- [ ] 支持用户自定义预览笔记的命令
- [ ] 支持补全，包括`noxe preview`自动补全`$NOXE_DIR`下的笔记名
- [ ] 彩色输出
- [x] 用户通过`$NOXE_DIR`下的`.ignore`或`.gitignore`文件指定忽略文件夹和文件
- [ ] 也许会考虑处理symlink
