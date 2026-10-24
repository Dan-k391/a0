# The official Lox test suite

`test/` is copied unchanged from <https://github.com/munificent/craftinginterpreters> at commit
`4a840f70f69c6ddd17cfef4f6964f8e1bcd8c3d4`, under the MIT license in `LICENSE`.

`tests/suite.rs` runs it the way the book's `tool/bin/test.dart` runs the `jlox` suite: same expectation
comments, same exit codes (65 for compile errors, 70 for runtime errors), same skip list.
