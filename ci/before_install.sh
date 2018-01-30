if [ $TRAVIS_OS_NAME = linux ]; then
    # Trusty uses pretty old versions => use newer

    # GStreamer
    curl -L https://people.freedesktop.org/~slomo/gstreamer.tar.gz | tar xz
    sed -i "s;prefix=/root/gstreamer;prefix=$PWD/gstreamer;g" $PWD/gstreamer/lib/pkgconfig/*.pc
    export PKG_CONFIG_PATH=$PWD/gstreamer/lib/pkgconfig
    export GST_PLUGIN_SYSTEM_PATH=$PWD/gstreamer/lib/gstreamer-1.0
    export GST_PLUGIN_SCANNER=$PWD/gstreamer/libexec/gstreamer-1.0/gst-plugin-scanner
    export PATH=$PATH:$PWD/gstreamer/bin
    export LD_LIBRARY_PATH=$PWD/gstreamer/lib:$LD_LIBRARY_PATH

    # GTK3
    WD="$PWD"
    cd $HOME
    curl -L https://github.com/gkoz/gtk-bootstrap/releases/download/gtk-3.18.1-2/deps.txz | tar xJ
    cd "$WD"
    export PKG_CONFIG_PATH="$HOME/local/lib/pkgconfig":$PKG_CONFIG_PATH
    export LD_LIBRARY_PATH="$HOME/local/lib/":$LD_LIBRARY_PATH
else
    brew update
    brew install gtk+3 gstreamer gst-plugins-base
fi
