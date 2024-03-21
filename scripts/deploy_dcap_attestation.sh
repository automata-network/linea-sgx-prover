#!/bin/bash -e

git submodule init
git submodule update
cd verifier/automata-dcap-v3-attestation

npm install
ls -l ./scripts/deploy.ts
npx hardhat run ./scripts/deploy.ts