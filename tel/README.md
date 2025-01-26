# Turbofuro Expression Language
Predictable expression language with familiar syntax. You can embed TEL in your application and evaluate user expressions in a safe way.

## Features
- Familiar C/Java/JavaScript like syntax
- Any JSON is a valid expression
- No object references  objects and arrays are compared by value (deep equality)
- Available to use on [crates.io](https://crates.io/crates/tel)
- Compiles to WebAssembly and is available as [npm package](https://www.npmjs.com/package/@turbofuro/tel-wasm)
- Value and store (assignment like) expressions

## Getting Started
Download repository from [GitHub](https://github.com/turbofuro/turbofuro). You will find the TEL implementation in the `tel` folder. The `tel-wasm` folder contains the WebAssembly bindings.

## WASM Build
TEL is available to use on web as a WebAssembly module, available on npm as part of [Omnitool](https://www.npmjs.com/package/@turbofuro/omnitool).

## Contributing
We welcome all contributions with 💛 

Feel free to create issues including those with feature suggestions. If you want to help, but not sure how, reach out to [@pr0gramista](https://github.com/pr0gramista) (Twitter/LinkedIn/email) directly.

## License
TEL and Turbofuro OSS parts are licensed under [Apache-2.0](https://opensource.org/license/apache-2-0/).

Happy Coding! 🚀
