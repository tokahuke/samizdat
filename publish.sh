#! /usr/bin/env bash

###
#
# Utility script for publishing a new version of Samizdat to the testbed.
#
###

export CURRENT_BRANCH=`git branch --show-current`

git push &&
git checkout stable &&
git merge main &&
git push &&
git checkout $CURRENT_BRANCH
