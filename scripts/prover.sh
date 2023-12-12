#!/bin/bash
source $(dirname $0)/executor.sh
if [[ "$NETWORK" == "" ]]; then
    NETWORK=localhost
fi

APP=$(basename $0 | awk -F. '{print $1}') execute $@
