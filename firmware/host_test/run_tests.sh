#!/bin/sh
# Host test for the firmware streaming patch decoder: builds the exact
# device sources (../main/patch_stream.c + ../main/tinfl.c) with the
# host compiler and runs generated protocol-v2 packets through them.
set -e
cd "$(dirname "$0")"
python3 gen_cases.py
cc -std=c11 -O2 -Wall -Wextra -I../main -o test_patch_stream \
    test_patch_stream.c ../main/patch_stream.c ../main/tinfl.c
./test_patch_stream cases
