# Archive regression fixtures

`encrypted-inner.7z`, `aes128-store.zip` and `aes256.zip` were generated with 7-Zip 24.05. They use the test password `nestedunpack-regression-only` and contain the synthetic payloads asserted by the tests. The split tests hand the decoder contiguous slices of the 7z fixture. The AES tests include a copy using AE-1 CRC metadata.

The `.rar` fixtures come from the rarfile v4.5 test suite and are covered by [LICENSE-rarfile.txt](LICENSE-rarfile.txt). [rarfile test fixtures](https://github.com/markokr/rarfile/tree/v4.5/test/files)
