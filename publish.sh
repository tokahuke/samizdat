#! /usr/bin/env bash

export CURRENT_BRANCH=`git branch --show-current`

git push &&
git checkout stable &&
git merge main &&
git push &&
git checkout $CURRENT_BRANCH
