#! /usr/bin/env bash

##
# Need envsubst?
#
# brew install gettext
# brew link --force gettext
#
##

echo "Creating tarball for homebrew"

pwd=$PWD
cd $OUTPUT &&
tar -czvf samizdat.tar.gz samizdat samizdat-node &&
cd $pwd &&

echo "Rendeing homebrew formula"

export SHA256SUM=$(sha256sum $OUTPUT/samizdat.tar.gz | cut -f 1 -d " ")

envsubst '$SAMIZDAT_PUBLIC_KEY,$VERSION,$SHA256SUM' \
    < Samizdat.rb \
    > ../../../../homebrew-samizdat/Samizdat.rb &&

echo "Commiting changes to homebrew repository (brew tap)"

cd ../../../../homebrew-samizdat &&
git add . &&
(
    git commit -m "Update Samizdat.rb for distribution" && git push || echo "Nothing to commit? Ok!"
) &&
cd ../samizdat/install/node/aarch64-apple-darwin
