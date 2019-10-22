set -x

if [ $TRAVIS_OS_NAME = linux ]; then
    # Xenial uses pretty old versions => use newer

    # GStreamer
    curl -L https://people.freedesktop.org/~slomo/gstreamer-1.14.3.tar.gz | tar xz
    sed -i "s;prefix=/root/gstreamer;prefix=$PWD/gstreamer;g" $PWD/gstreamer/lib/x86_64-linux-gnu/pkgconfig/*.pc
    export PKG_CONFIG_PATH=$PWD/gstreamer/lib/x86_64-linux-gnu/pkgconfig
    export LD_LIBRARY_PATH=$PWD/gstreamer/lib/x86_64-linux-gnu:$LD_LIBRARY_PATH
elif [ $TRAVIS_OS_NAME = osx ]; then
    brew update
    brew install gtk+3 gstreamer gst-plugins-base
else:
    echo Unknown OS $TRAVIS_OS_NAME
fi

set +x
