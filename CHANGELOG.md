## 0.1.27
* Omnitool: toolkit for Turbofuro
* Add streamable resource stack
* Add fuel and maxAsyncYield parameters to wasm/run_wasi  
* Add timeout parameter to actors/send
* Remove support for actors/send_command
* Add resource tracking
* Dropping resources that doesn't exist will now fail with an error
* Remove many dangerous unwraps
* Update to Rust 1.84

## 0.1.26
* Add support for decorators
* Add support for disabled steps
* Add binary/hex/base64 TEL functions
* Add Ollama module (experimental)
* Add Fantoccini module
* Add File System all-in-one byte write/read functions
* Add HMAC, SHA2 crypto
* JWT decoding algorithm is now configurable
* Add Lua module
* Add LibSQL and Postgres execute and drop connection functions

## 0.1.25
* Add support for `throwOnHttpError` parameter in HTTP functions
* Improved debug functions
* Add new file system functions `copy`, `canonicalize`, `create_directory`, `rename`, `remove_file`, `remove_directory`
* Fix panic when debug module can't be started
* Improve state reporting
* Deactivate stale debug sessions

## 0.1.24
* Add `replace` string function
* Add support for deep encoding of form `application/x-www-form-urlencoded` request bodies
* Improve image and libSQL error handling

## 0.1.23
* Support for libSQL and thus SQLite
* Support for basic image processing
* Update to Rust 1.82
* Add closing frame reason to WS disconnection handler

## 0.1.22
* Fix parsing objects with nullable properties
* Remove 'unknown' description

## 0.1.21
* Add support for `multipart` request body
* Add support for function annotations
* Add debug expiration
* Add debug state reporting
* Add hints for rental workers
* Fix Postgres not being able to handle nullable TIMESTAMPTZ columns
* Add more TEL functions
* Improve TEL predictions

## 0.1.20
* Improve Throw step
* Support for Parse and Transform steps
* Upgrade to Rust 1.81

## 0.1.19
* Re-release of 0.1.18 because of broken GitHub actions pipeline

## 0.1.18
* Debug actions
* Concurrent debug runs
* KV expiration and increment support
* Add get current date and time action
* Fixed PubSub subscription being cancelled when the actor lags behind

## 0.1.17
* Added support for HTTP cookies
* Added HTTP version to request object
* Add fs/read_dir native function
* Added support for debugging module starters
* Added basic tasks functions

## 0.1.16
* Added support for live debugging with module reloading
* Fixed environment reloading
* Added ability to create custom HTTP client
* Fixed and improved cloud agent
* Improved worker shutdown procedure

## 0.1.15
* Added support for live debugging  
* Worker status reporting
* Better error handling
* Code reorganization
* Added context parameter to PubSub subscription
* Added more data to execution reports
* Added --addr command line argument for setting the address to bind to
* Removed `saveAs` leftovers

## 0.1.14
* Added `mail` module for sending emails via SMTP
* Added `pretty` parameter to JSON stringify function
* Print worker version on start
* Added worker version to stats
* Fixed leaking Redis PubSub

## 0.1.13
* Added `form` parameter to HTTP request function
* Added new HTTP client functions
* Added form data functions
* Added concept of Streams
* Added request/reply messaging to actors
* Worker is now throwing 500 errors immediately when actor fails to run to response to HTTP request
* Add URL parsing utilities

## 0.1.12
* Fix panics when Postgres row contains NULL
* Upgraded TEL

## 0.1.11
* Upgraded dependencies to axum 0.7.5, hyper 1.0
* Security fixes
* CRON schedule now uses more common format
* Improved Postgres data type conversion
* Added more specialized base descriptions like string.uuid

## 0.1.10

## 0.1.3
* Add option to specify custom cloud URL and operator URL

## 0.1.2
* Fixes in Docker image dependencies of libc and OpenSSL

## 0.1.1
* Fixes to GitHub Actions workflow

## 0.1.0
* First public release