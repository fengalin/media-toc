# Media-TOC
Media-TOC is an application to build a table of contents from of a media file or
to split a media file into chapters.

**Important**: Media-TOC is in a very early stage of development. Don't expect
anything usable anytime soon. Of course, you can [contribute](#contribute) to the project
if you find it interesting.

# Design
## Technologies
Media-TOC is developped in Rust. This is my first project using this language.
Current design is merely a [UI prototype](#ui) and a set of technologies:
- [GTK-3](https://developer.gnome.org/gtk3/stable/) and [Glade](https://glade.gnome.org/).
- [FFMPEG](https://ffmpeg.org/).

## <a name='ui'></a>UI prototype
![Media TOC UI prototype](assets/media-toc.png)

# <a name='contribute'></a>How to contribute
Contributions are welcomed.
- For a design or feature proposal or a bug report, you can [declare an issue](https://github.com/fengalin/media-toc/issues).
- If you wish to contribute to the code, please fork your own copy and submit a
[pull request](https://github.com/fengalin/media-toc/pulls).

# Generation
## Dependencies
Dependencies are handled by [Cargo](http://doc.crates.io/). You will need the
following packages installed on your OS:

### Fedora
```
$ sudo dnf install gtk3-devel glib2-devel
```

### Debian & Unbuntu
```
$ sudo apt-get install libgtk-3-dev
```

### OS X
```
$ brew install gtk+3
```

### Windows
See [this page](http://gtk-rs.org/docs/requirements.html).

# Build and run
Use Cargo (from the root of the project directory):
```
$ cargo run
```

