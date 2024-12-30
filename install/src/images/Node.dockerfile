FROM node:current-alpine

WORKDIR /build
COPY . .

ENTRYPOINT [ "/bin/ash" ]
