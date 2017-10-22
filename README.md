# media-toc [![Build Status](https://travis-ci.org/fengalin/media-toc.svg?branch=master)](https://travis-ci.org/fengalin/media-toc) [![Build status](https://ci.appveyor.com/api/projects/status/eu9p6ggcflj89h3v?svg=true)](https://ci.appveyor.com/project/fengalin/media-toc)
**media-toc** is an application to build a table of contents from a media file or
to split a media file into chapters. It is primarily developed in Rust on Linux,
it runs on Windows and should also work on macOS.

**media-toc** is not fully functional yet, see the [Status section](#status) below.
Of course, you can contribute to the project if you find it interesting.

## <a name='status'></a>Status
At the moment, **media-toc** can:
- Open a media file: display metadata from the media, the cover image, the first
video frame, the chapters list and marks at the beginning of each chapter on the
timeline.
- Play/Pause the audio and video, draw the audio waveform and select current
chapter in the list while playing.
- Seek in the media by clicking on the waveform, on the timeline or in the
chapters list.
- Zoom in/out the waveform on the time axis.
- Add/remove a chapter. Note that you can't export the result yet.

## <a name='ui'></a>Screenshots
### UI with a video file
![media-toc UI Video](assets/media-toc_video.png)

### UI with an audio file
![media-toc UI Audio](assets/media-toc_audio.png)

### Waveform showing 5.1 audio channels
![media-toc Waveform 5.1 audio channels](assets/waveform_5.1_audio_channels.png)

# Technologies
**media-toc** is developed in Rust and uses the following technologies:
- **GTK-3** ([official documentation](https://developer.gnome.org/gtk3/stable/),
[Rust binding](http://gtk-rs.org/docs/gtk/)) and [Glade](https://glade.gnome.org/).
- **Cairo** ([official documentation](https://www.cairographics.org/documentation/),
[Rust binding](http://gtk-rs.org/docs/cairo/index.html)).
- **GStreamer** ([official documentation](https://gstreamer.freedesktop.org/documentation/),
[Rust binding](https://github.com/sdroege/gstreamer-rs)).

# Environment preparation
## Toolchain
Rust nightly version is required at the moment.
```
$ curl https://sh.rustup.rs -sSf | sh
```
Select the nightly toolchain. See the full documentation
[here](https://github.com/rust-lang-nursery/rustup.rs#installation).

## Dependencies
Rust dependencies are handled by [Cargo](http://doc.crates.io/). You will also
need the following packages installed on your OS:

### Fedora
```
sudo dnf install gtk3-devel glib2-devel gstreamer1-devel gstreamer1-plugins-base-devel gstreamer1-plugins-good gstreamer1-plugins-bad-free-gtk
```

### Debian & Unbuntu
```
sudo apt-get install libgtk-3-dev libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev gstreamer1.0-plugins-good gstreamer1.0-plugins-bad
```

### MacOS
*Needs confirmation*
```
brew install gtk+3 gstreamer-1.0-devel gstreamer-1.0-plugins-good gstreamer-1.0-plugins-bad
```

### Windows
- MSYS2: follow [this guide](http://www.msys2.org/).
- Install the development toolchain, GTK and GStreamer<br>
Note: for a 32bits system, use `mingw-w64-i686-...`
```
pacman -S mingw-w64-x86_64-toolchain base-devel mingw-w64-x86_64-gtk3 \
    mingw-w64-x86_64-gstreamer mingw-w64-x86_64-gst-plugins-base
```
For the execution, you will also need at least (other packages might be
necessary for specific codecs):
```
pacman -S mingw-w64-x86_64-gst-plugins-good mingw-w64-x86_64-gst-plugins-bad \
    mingw-w64-x86_64-gst-plugins-ugly
```

- Launch the [rustup installer](https://www.rustup.rs/).
When asked for the default host triple, select `x86_64-pc-windows-gnu` (or
`i686-pc-windows-gnu` for a 32bits system), then select `nightly`.
- From a MSYS2 mingw shell
  - add cargo to the `PATH`:
  ```
  echo 'PATH=$PATH:/c/Users/'$USER'/.cargo/bin' >> .bashrc
  ```
  - Restart the MSYS2 shell before using `cargo`.

# Build and run
Use Cargo (from the root of the project directory):
```
$ cargo run --release
```
