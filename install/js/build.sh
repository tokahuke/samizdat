#! /usr/bin/env bash

npm install && 
npm run build &&
cp ./dist/samizdat.js $OUTPUT
