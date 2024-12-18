#! /usr/bin/env bash

###
#
# Builds the Samizdat installers for a given set of architectures (in `$1`; all
# architecture if not set).
#
###

set -e

# Do not run this in any branch other than stable:
if [ $(git branch --show-current) != 'stable' ]
then
    echo 'Not in `stable` branch. Stopping...'
    exit 1
fi

# Deciding which architectures should be installed.
if [ -z $1 ]; then
    archs=$(ls ./install/node) # All architecture from Node, if not set.
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


# This is the version name that is going to be the output directory prefix.
version="latest"

# Clean the `dist` directory.
rm -rf ./dist
mkdir -p ./dist


# Pesky configurations for MacOS.
if [ $(uname) == "Darwin" ]
then
    echo
    echo "Darwin host detected"
    export CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=x86_64-unknown-linux-gnu-gcc
    export CC_x86_64_unknown_linux_gnu=x86_64-unknown-linux-gnu-gcc
    export CXX_x86_64_unknown_linux_gnu=x86_64-unknown-linux-gnu-g++
    # export TARGET_CC=x86_64-linux-musl-gcc
fi

for arch in $archs
do
    if [ ! -f ./install/node/$arch/.skip ]
    then
        echo
        echo "Starting to build node installer for $arch"
        echo

        output=./dist/$version/node/$arch/
        
        mkdir -p $output

        cargo build --release --bin samizdat-node --target $arch
        cargo build --release --bin samizdat --target $arch

        for artifact in $(cat ./install/node/$arch/artifacts.txt)
        do
            echo "Loading artifact $artifact"
            if [ $artifact != "" ]
            then
                cp ./target/$arch/release/$artifact $output || \
                    (echo "no such artifact $artifact in $arch" && exit 1);
            fi
        done

        cd ./install/node/$arch/
        OUTPUT=../../../$output VERSION=$version . ./build.sh
        cd ../../../
    else
        echo
        echo "No build routine for node in $arch! Skiping..."
    fi
done


for arch in $archs
do
    if [ -f ./install/hub/$arch/build.sh ]
    then
        echo
        echo "Starting to build hub installer for $arch"
        echo

        output=./dist/$version/hub/$arch/
        
        mkdir -p $output

        cargo build --release --bin samizdat-hub --target $arch

        for artifact in $(cat ./install/node/$arch/artifacts.txt)
        do
            echo "Loading artifact $artifact"
            if [ $artifact != "" ]
            then
                cp ./target/$arch/release/$artifact $output || \
                    (echo "no such artifact $artifact in $arch" && exit 1);
            fi
        done

        cd ./install/hub/$arch/
        OUTPUT=../../../$output VERSION=$version . ./build.sh
        cd ../../../ 
    else
        echo
        echo "No build routine for hub in $arch! Skiping..."
    fi
done

echo
echo "Starting to build SamizdatJS"
echo

cd js
output=../dist/$version/js/
mkdir -p $output
OUTPUT=$output . ../install/js/build.sh
cd /..
