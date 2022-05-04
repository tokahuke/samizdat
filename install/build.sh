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

for arch in $archs
do
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
done
