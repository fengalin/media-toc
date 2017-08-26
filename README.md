# Media-TOC
Media-TOC is an application to build a table of contents from a media file or
to split a media file into chapters.

**Important**: Media-TOC is in an early stage of development. Don't expect
anything usable soon. Of course, you can [contribute](#contribute) to the project
if you find it interesting.

## Status
At the moment, **Media-TOC** can:
- Open a media file (audio, video - and image but that doesn't make much sense).
- Display the video frame
- Display the cover image if available.
- Display metadata from the media.
- Display the audio wave form.
- Display chapters' data.
- Play the media, draw the video, the audio waveform and play the audio.

## <a name='ui'></a>Screenshots
![Media-TOC UI Video](assets/media-toc_video.png)
![Media-TOC UI Audio](assets/media-toc_audio.png)

# <a name='contribute'></a>How to contribute
Contributions are welcomed.
- For a design or feature proposal or a bug report, you can [declare an issue](https://github.com/fengalin/media-toc/issues).
- If you wish to contribute to the code, please fork your own copy and submit a
[pull request](https://github.com/fengalin/media-toc/pulls).

# Design
## Technologies
**Media-TOC** is developped in Rust and uses the following technologies:
- **GTK-3** ([official documentation](https://developer.gnome.org/gtk3/stable/),
[Rust binding](https://crates.io/crates/gtk)) and [Glade](https://glade.gnome.org/).
- **Cairo** ([official documentation](https://www.cairographics.org/documentation/),
[Rust binding](https://crates.io/crates/cairo-rs)).
- **GStreamer** ([official documentation](https://gstreamer.freedesktop.org/documentation/),
[Rust binding](https://github.com/sdroege/gstreamer-rs)).

# Environment preparation
## Toolchain
The nightly version is required for [this feature](https://doc.rust-lang.org/std/option/enum.Option.html#method.get_or_insert).

```
$ curl https://sh.rustup.rs -sSf | sh
```
Select the nightly toolchain. See the full documentation
[here](https://github.com/rust-lang-nursery/rustup.rs#installation).

## Dependencies
Rust dependencies are handled by [Cargo](http://doc.crates.io/). You will also need the
following packages installed on your OS:

### Fedora
```
$ sudo dnf install gtk3-devel glib2-devel gstreamer1-devel gstreamer1-plugins-base-devel
```

### Debian & Unbuntu
*Need confirmation*
```
$ sudo apt-get install libgtk-3-dev libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev
```

### MacOS
*Need confirmation*
```
$ brew install gtk+3 gstreamer-1.0-devel
```

### Windows (WIP)
Note: I could build `media-toc` successfully, but it fails to run.

- MSYS2: follow [this guide](http://www.msys2.org/).
- Install the development toolchain, GTK and GStreamer<br>
Note: for a 32bits system, use `mingw-w64-i686-...`
```
pacman -S mingw-w64-x86_64-toolchain base-devel mingw-w64-x86_64-gtk3 \
    mingw-w64-x86_64-gstreamer mingw-w64-x86_64-gst-plugins-base
```
- Rustup: launch the [rustup installer](https://www.rustup.rs/).
When asked for the default host triple, select `x86_64-pc-windows-gnu` (or
`i686-pc-windows-gnu` for a 32bits system), then select `nightly`.
- From a MSYS2 shell
  - add cargo to the `PATH`:
  ```
  echo 'PATH=$PATH:/c/Users/'$USER'/.cargo/bin' >> .bashrc
  ```
  - Create the file /c/Users/$USER/.cargo/config and add the following lines:<br>
  Note: on a 32bits system, use `i686-pc-windows-gnu` and `/msys32/mingw32/...`
  ```
  [target.x86_64-pc-windows-gnu]
  linker = "/msys64/mingw64/bin/gcc"
  ar = "/msys64/mingw64/bin/ar"
  ```
  - Restart the MSYS2 shell before using `cargo`.

# Build and run
Use Cargo (from the root of the project directory):
```
$ cargo run --release
```
