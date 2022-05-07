#! /usr/bin/env bash

if [ -z $1 ]; then
    archs=$(ls ./install/node)
else
    archs=$1
fi

echo 
echo "Starting to build installers for architectures:"
echo 

for arch in $archs
do
    echo "    - $arch"
done


version="latest"

rm -rf ./dist &&
mkdir -p ./dist &&


if [ $(uname) == "Darwin" ]; then
    echo
    echo "Darwin host detected"
    export CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=x86_64-unknown-linux-gnu-gcc
    export CC_x86_64_unknown_linux_gnu=x86_64-unknown-linux-gnu-gcc
    export CXX_x86_64_unknown_linux_gnu=x86_64-unknown-linux-gnu-g++
    # export TARGET_CC=x86_64-linux-musl-gcc
fi

for arch in $archs
do
    if [ -f ./install/node/$arch/build.sh ]; then
        echo
        echo "Starting to build node installer for $arch"
        echo

        output=./dist/$version/node/$arch/
        
        mkdir -p $output &&

        cargo build --release --bin samizdat-node --target $arch &&
        cargo build --release --bin samizdat --target $arch &&

        cp ./target/$arch/release/samizdat-node $output &&
        cp ./target/$arch/release/samizdat $output &&
        cd ./install/node/$arch/ &&
        OUTPUT=../../../$output VERSION=$version . ./build.sh &&
        cd ../../../
    else
        echo
        echo "No build routine for node in $arch! Skiping..."
    fi
done

for arch in $archs
do
    if [ -f ./install/hub/$arch/build.sh ]; then
        echo
        echo "Starting to build hub installer for $arch"
        echo

        output=./dist/$version/hub/$arch/
        
        mkdir -p $output &&

        cargo build --release --bin samizdat-hub --target $arch &&

        cp ./target/$arch/release/samizdat-hub $output &&
        cd ./install/hub/$arch/ &&
        OUTPUT=../../../$output VERSION=$version . ./build.sh &&
        cd ../../../
    else
        echo
        echo "No build routine for hub in $arch! Skiping..."
    fi
done

echo
echo "Starting to build SamizdatJS"
echo

cd js &&
output=../dist/$version/js/
mkdir -p $output &&
OUTPUT=$output . ../install/js/build.sh &&
cd /..

