# media-toc [![Build Status](https://travis-ci.org/fengalin/media-toc.svg?branch=master)](https://travis-ci.org/fengalin/media-toc) [![Build status](https://ci.appveyor.com/api/projects/status/eu9p6ggcflj89h3v?svg=true)](https://ci.appveyor.com/project/fengalin/media-toc)
**media-toc** is an application to build a table of contents from a media file or to split a media
file into chapters. It is primarily developed in Rust on Linux, it runs on Windows and should also
work on macOS.

**media-toc** is not complete yet, see the [Status section](#status) below.
Of course, you can contribute to the project if you find it interesting.

## <a name='ui'></a>Screenshots
### UI with a video file
![media-toc UI Video](assets/media-toc_video.png)

### UI with an audio file
![media-toc UI Audio](assets/media-toc_audio.png)

### Waveform showing 5.1 audio channels
![media-toc Waveform 5.1 audio channels](assets/waveform_5.1_audio_channels.png)

## <a name='status'></a>Status
At the moment, **media-toc** can:

- Open a media file: display metadata from the media, the cover image, the first video frame,
the chapters list and marks at the beginning of each chapter on the timeline.
- Play/Pause the audio and video, draw the audio waveform and select current chapter in the list
while playing.
- Seek in the media by left clicking on the waveform, on the timeline or in the chapters list.
- Play from a position until the end of current time frame by right clicking on the waveform
at the starting position in paused mode.
- Zoom in/out the waveform on the time axis.
- Add/remove a chapter.
- Export the table of contents to:
	* A Matroska container. Currently, this is possible only if the input streams are compatible
	with Matroska containers. I'll add an UI to allow converting streams later. This also requires
	this [patch on `gst-plugins-good`](https://bugzilla.gnome.org/show_bug.cgi?id=790686)
	to reactivate table of contents handling in `matroskamux`.
	* [mkvmerge simple chapter format](https://mkvtoolnix.download/doc/mkvmerge.html#mkvmerge.chapters).
	* [Cue Sheet](http://wiki.hydrogenaud.io/index.php?title=Cue_sheet).
- Split the media file's first audio stream into `flac`, `wave`, `opus`, `vorbis` or `mp3` files:
one file per chapter. I'll add an UI to allow selecting the stream to split later.
- Import the table of contents from:
	* A Matroska container.
	* [mkvmerge simple chapter format](https://mkvtoolnix.download/doc/mkvmerge.html#mkvmerge.chapters).

# How-to

## <a name='how-to-create-the-toc'></a>Create the table of contents

1. Click on the folder icon or the play icon to open the file selection dialog.
2. Select the mkv media for which you want to add a table of contents.
3. If you want to add a chapter starting at the begining of the file, you can click on the `+` icon
under the tree view at the bottom right of the window. The end of the chapter will match the end
of the media. This will change automatically if you add new chapters.
4. Click in the newly added chapter title column and fill the title for this chapter.
5. Play the stream until the next chapter's starting position. You can use the timeline to seek
in the media.
6. In order to precisely define the start of the new chapter, pause the media by clicking on the
play/pause button, then use the `+` button next to the waveform to zoom in. You can then seek around
current sample by clicking on the waveform. You can also play the media in current time frame by
right clicking at the starting position. Use the zoom, left click and right click until you reach
the position for the chapter to add.
7. When the cursor (the vertical yellow bar) matches the start of the chapter to add, click on the
`+` icon under the tree view at the bottom right of the window.
8. Click in the newly added chapter title column and fill a title for this chapter.
9. Go back to step 5 if you wish to add another chapter.

## <a name='how-to-save-the-toc'></a>Save the table of contents

You can export a table of contents to the `mkvmerge simple chapter format` which is a text file.
This file will be stored in the same folder as the original media and will be automatically loaded
next time you open this media.

1. Define the chapters as explained in [this how-to](#how-to-create-the-toc).
2. Click on the export button (the "gears" icon) on the right side of the header bar.
3. Select `mkvmerge text format`.
4. Click on `Export`. When the export is complete, a new file with the same name as your media and
with a `txt` extension will be created in the media's folder.

## Export the resulting media with its table of contents in a Matroska container

Currently, this is possible only for media with streams compatible with Matroska containers.
Warning: this also requires this [patch on `gst-plugins-good`](https://bugzilla.gnome.org/show_bug.cgi?id=790686).

1. Open a media with a table of contents, define the chapters as explained in [this how-to](#how-to-create-the-toc)
or open a media for which you already defined a table of contents (see [this how-to](#how-to-save-the-toc)).
2. Click on the export button (the "gears" icon) on the right side of the header bar.
3. Select `Matroska Container`.
4. Click on `Export`. When the export is complete, a new file with the same name as your media and
ending with `.toc.mkv` will be created in the media's folder.

## Split the audio stream into one file per chapter

Currently, `media-toc` can only export the first audio stream.

1. Open a media with a table of contents, define the chapters as explained in [this how-to](#how-to-create-the-toc)
or open a media for which you already defined a table of contents (see [this how-to](#how-to-save-the-toc)).
2. Click on the export button (the "gears" icon) on the right side of the header bar.
3. Select `Split into one file per chapter`.
4. Select to desired output format: `flac`, `wave`, `opus`, `vorbis` or `mp3`.
5. Click on `Export`. When the export is complete, audio files will be created in the media's folder.
The files are named after the artist, media title, chapter number and chapter title.

## Use `mkvmerge` to add the toc to an existing Matrsoka media

Currently, exporting the table of contents to a Matroska container requires a patched version of
`gst-plugins-good`. If you don't know how to patch `gst-plugins-good`, you can follow these
instructions to use `mkvmerge`:

1. Install `mkvtoolnix` using your package manager.
2. Export your table of contents to the `mkvmerge simple chapter format` (see [this how-to](#how-to-save-the-toc)).
3. Open a terminal and `cd` to the directory where your Matroska file is located.
4. Issue the following command (where _media_ is the name of your mkv file without the extension):
    ```
    mkvmerge --chapters _media_.txt -o output_file.mkv _media_.mkv
    ```

The file `output_file.mkv` will now contain the media with the chapters you defined.

# Technologies
**media-toc** is developed in Rust and uses the following technologies:
- **GTK-3** ([official documentation](https://developer.gnome.org/gtk3/stable/),
[Rust binding](http://gtk-rs.org/docs/gtk/)) and [Glade](https://glade.gnome.org/).
- **Cairo** ([official documentation](https://www.cairographics.org/documentation/),
[Rust binding](http://gtk-rs.org/docs/cairo/index.html)).
- **GStreamer** ([official documentation](https://gstreamer.freedesktop.org/documentation/),
[Rust binding](https://sdroege.github.io/rustdoc/gstreamer/gstreamer/)).

# <a name='generation'></a>Generation
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
sudo dnf install gtk3-devel glib2-devel gstreamer1-devel \
	gstreamer1-plugins-base-devel gstreamer1-plugins-good \
	gstreamer1-plugins-bad-free gstreamer1-plugins-ugly-free \
	gstreamer1-libav
```

### Debian & Unbuntu
```
sudo apt-get install libgtk-3-dev libgstreamer1.0-dev \
	libgstreamer-plugins-base1.0-dev gstreamer1.0-plugins-good \
	gstreamer1.0-plugins-bad gstreamer1.0-plugins-ugly \
	gstreamer1.0-libav
```

### macOS
*Needs confirmation*
```
brew install gtk+3 gstreamer gst-plugins-base gst-plugins-good \
	gst-plugins-bad gst-plugins-ugly gst-libav
```

### Windows
- MSYS2: follow [this guide](http://www.msys2.org/).
- Install the development toolchain, GTK and GStreamer<br>
Note: for a 32bits system, use `mingw-w64-i686-...`
```
pacman --noconfirm -S mingw-w64-x86_64-gtk3 \
	mingw-w64-x86_64-gstreamer mingw-w64-x86_64-gst-plugins-base \
	mingw-w64-x86_64-gst-plugins-good mingw-w64-x86_64-gst-plugins-bad \
	mingw-w64-x86_64-gst-plugins-ugly mingw-w64-x86_64-gst-libav
```

- Launch the [rustup installer](https://www.rustup.rs/).
When asked for the default host triple, select `x86_64-pc-windows-gnu` (or
`i686-pc-windows-gnu` for a 32bits system), then select `nightly`.
- From a MSYS2 mingw shell
  - add cargo to the `PATH`:
  ```
  echo 'PATH=$PATH:/c/Users/'$USER'/.cargo/bin' >> /c/Users/'$USER'/.bashrc
  ```
  - Restart the MSYS2 shell before using `cargo`.

# Build and run
Use Cargo (from the root of the project directory):
```
$ cargo run --release
```
