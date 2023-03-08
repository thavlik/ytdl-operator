#!/bin/bash
cd $(dirname $0)/..
fmt() {
    pushd $1
        rustfmt --edition 2021 src/*.rs
    popd
}
fmt common
fmt executor
fmt operator
fmt types