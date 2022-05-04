#! /usr/bin/env bash

##
# Need envsubst?
#
# brew install gettext
# brew link --force gettext
#
# 
# You will also need this for code signing:
#
# brew install mitchellh/gon/gon
##

envsubst '$SAMIZDAT_PUBLIC_KEY,$VERSION' \
    < Samizdat.rb \
    > ../../../../homebrew-samizdat/Samizdat.rb &&
cp samizdat-node.plist $OUTPUT &&

echo "Creating tarball for homebrew"

tar -czvf $OUTPUT/samizdat.tar.gz $OUTPUT/samizdat $OUTPUT/samizdat-node &&

echo "Commiting changes to homebrew repository (brew tap)"

cd ../../../../homebrew-samizdat &&
git add . &&
(git commit -m "Update Samizdat.rb for distribution" || echo "Nothing to commit? Ok!") &&
git push &&
cd ../samizdat/install/node/aarch64-apple-darwin
