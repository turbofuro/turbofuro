## 0.1.18 (pending)
* Debug actions
* Concurrent debug runs
* KV expiration and increment support
* Get current date and time

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