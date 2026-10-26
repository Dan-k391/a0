//! The codes of the errors and warnings A0 reports to the people who write programs, with an
//! explanation of each (`lang explain E0201`).
//!
//! Every kind of problem has its own code, and a code never changes meaning. `E0xxx` are syntax
//! errors, `E1xxx` names, `E2xxx` types, `E3xxx` control flow, `E4xxx` the entry point, `E5xxx`
//! packages and `Wxxxx` warnings. Problems that only a language's own code can cause (a malformed
//! tree, a misused builder) keep descriptive codes such as `hir.tree`; they are bugs to report to
//! the language's authors, not errors in a program.
//!
//! The explanations speak of concepts, not syntax, since each language writes them its own way.

/// One code and what it means.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct CodeInfo {
    /// The code, as `E2001`.
    pub code: &'static str,
    /// A short description, as in the codes' list.
    pub title: &'static str,
    /// What the problem is and how to fix it, as `explain` prints it.
    pub explanation: &'static str,
}

macro_rules! codes {
    ($($name:ident = $code:literal, $title:literal, $explanation:literal;)*) => {
        $(
            #[doc = $title]
            pub const $name: &str = $code;
        )*

        /// Every code, in order.
        pub const ALL: &[CodeInfo] = &[$(CodeInfo { code: $code, title: $title, explanation: $explanation }),*];
    };
}

codes! {
    SYNTAX = "E0001", "syntax error",
        "The program does not follow the language's grammar here: a token is missing or one appears where it cannot. The message says what the parser expected.";
    INVALID_TOKEN = "E0002", "invalid token",
        "These characters do not form any token of the language, for example an unknown symbol or a string that is never closed.";
    TOO_DEEP = "E0003", "nested too deeply",
        "The program nests deeper than the compiler goes: expressions inside expressions or blocks inside blocks, more than 10,000 levels (in a long chain of operators, each operator is a level). Name intermediate results with variables, or move inner parts into functions of their own.";
    NEWER_FORMAT = "E0004", "written for a newer A0",
        "The text says it is in a later version of its format than this A0 reads: a module of HIR or core IR text (`module \"m\" version 2;`) or a grammar (`grammar g version 2;`). Each A0 1.x reads version 1 of every format, which is also what text without a version is. Use the newer A0 that wrote the text, or write it in the version this one reads.";

    UNKNOWN_NAME = "E1001", "unknown name",
        "The name is not declared where it is used: no variable, function or global of that name is in scope. It may be misspelled, declared later in an inner block, or declared in a block that has ended.";
    UNKNOWN_TYPE = "E1002", "unknown type",
        "No type of this name is declared. Check the spelling, and that the type is declared somewhere in the program.";
    UNKNOWN_FIELD = "E1003", "unknown field",
        "The value's type has no field of this name. The message names the type; its declaration lists the fields it has.";
    UNKNOWN_VARIANT = "E1004", "unknown variant",
        "The enumeration has no variant of this name. Its declaration lists the variants it has.";
    DUPLICATE = "E1005", "declared twice",
        "Two declarations in the same place have the same name, so a use of the name could not tell them apart. Rename or remove one of them; the message points at both.";
    FIELD_TWICE = "E1006", "field given twice",
        "A value of a structure gives the same field twice. Each field is given exactly once.";
    MISSING_FIELDS = "E1007", "missing fields",
        "A value of a structure must give every field of the structure. The message lists the fields that are missing.";
    NOT_A_VALUE = "E1008", "not a value",
        "The name refers to something that can be called but not used as a value, such as a function written in C.";
    NOT_ASSIGNABLE = "E1009", "cannot assign",
        "Only variables, globals, fields and elements can be assigned; a function's name cannot.";
    UNKNOWN_MODULE = "E1010", "unknown module",
        "No module of this name is imported here, or no file holds it. A module is imported by the file that uses it, and its file is found from the module's name.";
    PRIVATE = "E1011", "private item",
        "The item belongs to another module and is not public there, so only that module can use it. Make it public in its module, or use something that is.";
    IMPORT_CYCLE = "E1012", "import cycle",
        "Modules import each other in a circle, so none of them can be compiled before the others. Move what they share into a module that both import.";
    UNKNOWN_METHOD = "E1013", "unknown method",
        "The value's type has no method of this name: its type declares none, no interface it is used as requires one, and no field of that name holds a function.";
    INLINE_FIELD = "E1015", "inline struct holds a reference",
        "An inline struct lives where it is used (in a variable, a field, an array element) and is copied as its bytes, so it can hold only numbers, booleans and other inline structs, which the collector need not see. Use a struct that is not inline to hold strings, arrays or objects.";
    INLINE_CYCLE = "E1016", "inline struct contains itself",
        "An inline struct holds its fields in itself, so it cannot contain itself, directly or through other inline structs: it would have no end. Hold the inner value through a struct that is not inline.";

    MISMATCHED_TYPES = "E2001", "mismatched types",
        "A value has a different type than the place it is used requires. The message says which type was expected and which was found, and points at where the expectation comes from: a declared type, a parameter, a field or a function's result.";
    ARGUMENT_COUNT = "E2002", "wrong number of arguments",
        "A function, closure or variant is given a different number of arguments than it takes. The message points at its declaration.";
    NOT_CALLABLE = "E2003", "not a function",
        "Only functions and closures can be called. The value called here has another type.";
    BAD_OPERAND = "E2004", "operator not defined for this type",
        "The operator is not defined for values of this type; arithmetic needs numbers, for example.";
    CANNOT_INFER = "E2005", "cannot infer a type",
        "Nothing in the program determines this type: for example an empty collection that is never filled, or a value only ever compared with null. Declaring the type of the variable resolves it.";
    NOT_NULLABLE = "E2006", "not nullable",
        "This value can never be null, so it cannot be tested for null or unwrapped.";
    MAYBE_NULL = "E2007", "may be null",
        "This value may be null, so it cannot be used as if it were present. Check that it is not null first, which narrows its type, or unwrap it if it is known to be present.";
    BAD_CONVERSION = "E2008", "invalid conversion",
        "There is no conversion between these two types.";
    BAD_TYPE_TEST = "E2009", "invalid type test",
        "A dynamic value can only be tested for a type that a dynamic value can hold: a primitive, a structure or an enumeration.";
    NULL_NOT_ALLOWED = "E2010", "null where no null is allowed",
        "Null is used where the type does not allow it; only nullable types hold null.";
    VALUE_NOT_NULLABLE = "E2011", "nullable value type",
        "Only references (strings, collections, structures, enumerations, closures) can be nullable; numbers and booleans cannot.";
    NOT_INDEXABLE = "E2012", "cannot index",
        "Only arrays and maps can be indexed.";
    NOT_AN_ENUM = "E2013", "not an enumeration",
        "Only values of an enumeration can be matched against its variants.";
    NON_EXHAUSTIVE = "E2014", "match does not cover every variant",
        "A match must handle every variant of the enumeration, or have a catch-all arm. The message lists the variants that are not covered.";
    BAD_RESULT = "E2015", "wrong result",
        "The function's body ends, or returns, without a value of the function's result type.";
    TYPE_ARGUMENT_COUNT = "E2016", "wrong number of type arguments",
        "A generic function or type is given a different number of type arguments than it takes.";
    BAD_MAP_KEY = "E2017", "invalid map key",
        "Map keys must be integers, booleans or strings.";
    NO_FIELDS = "E2018", "not a structure",
        "Only structures have fields; this value has another type.";
    BAD_GENERIC_USE = "E2019", "invalid use of a type parameter",
        "Inside a generic function or type, a type parameter stands for any type, so its values can only be stored, passed, compared and converted, not used as numbers or strings.";
    CANNOT_PROPAGATE = "E2020", "cannot pass the failure on",
        "Passing a failure on (`?` in many languages) takes a value that is either nullable or of an enumeration whose first variant holds the result, such as `Result<T, E>`, and returns the null or the failure from the function. So the function must return a nullable value, or the same enumeration.";
    NOT_IMPLEMENTED = "E2021", "interface not implemented",
        "A type is used as an interface it does not implement, or declares that it implements one without having all of its methods with the signatures the interface gives them (`Self` standing for the type).";
    NOT_DYNAMIC = "E2022", "not callable through an interface value",
        "The method takes or returns `Self`, the type that implements the interface, which a value of the interface type does not know. Call it on a value whose type is known, or on a value of a type parameter bounded by the interface.";
    LITERAL_RANGE = "E2023", "literal out of range",
        "A number written in the program takes the integer type expected of it, and must fit in it: an 8-bit unsigned integer holds 0 to 255, a signed one -128 to 127. Write a number that fits, or convert a value of a wider type with a conversion, which wraps.";
    INLINE_VALUE = "E2024", "not possible with an inline value",
        "An inline struct's value is its bytes, with no identity and no type information of its own, so it cannot be compared with `==`, held as an `any` or a map's value, or used as an interface. Compare its fields, or use a struct that is not inline.";
    NO_ADDRESS = "E2025", "cannot take the address",
        "A pointer can point at a variable, a field or an element that holds plain data: a number, a boolean, a pointer or an inline struct. Other values (references, `any`) have no address a pointer can hold, and a value that is not stored anywhere has none at all. Store it in a variable first.";
    MANUAL_ONLY = "E2026", "only without the collector",
        "Freeing memory is for programs that manage it themselves (`memory manual`): with the collector, an object lives as long as something refers to it, and freeing it would leave those references pointing at nothing.";

    NEEDS_UNSAFE = "E2027", "needs `unsafe`",
        "Raw pointers are unchecked: reading or writing through one, or taking or moving an address, can touch memory that is not what the program thinks it is. A language that offers them (B0) allows them only inside an `unsafe` block, which marks the code to check by hand. Move the operation into `unsafe { ... }`, or use arrays, which are checked.";

    OUTSIDE_LOOP = "E3001", "outside a loop",
        "Leaving or continuing a loop is only possible inside a loop.";

    NO_ENTRY = "E4001", "no entry point",
        "A program needs a `main` function, which is where it starts.";
    BAD_ENTRY = "E4002", "invalid entry point",
        "The `main` function takes no parameters and returns nothing or an integer, the program's exit status.";

    MANIFEST_SYNTAX = "E5001", "invalid manifest",
        "The package's manifest is not valid TOML of the kind it uses: tables in brackets, `key = value` lines, values that are strings in double quotes, numbers, booleans, arrays or inline tables in braces.";
    MANIFEST_KEY = "E5002", "invalid manifest entry",
        "The manifest has a table, a key or a value the package format does not know, or lacks one it needs. `[package]` takes `name` (required), `version`, `main` and `lib`; `[dependencies]` gives each dependency `path = \"...\"` or `git = \"...\"` with an optional `branch`, `tag` or `rev`.";
    DEPENDENCY_NOT_FOUND = "E5003", "dependency not found",
        "A dependency names a directory with no package in it (no manifest), or a package whose library file is missing. A path is relative to the directory of the manifest that names it.";
    DEPENDENCY_FETCH = "E5004", "dependency cannot be fetched",
        "Fetching a git dependency failed: git is not installed, the repository cannot be reached, or it has no such branch, tag or commit. The message has git's own words.";
    LOCK_OUT_OF_DATE = "E5005", "lock file out of date",
        "The build was asked to use the lock file as it is (`--locked`), but the lock file does not pin every git dependency the manifest names. Build once without `--locked` to update it, and commit it.";
    NO_PACKAGE = "E5006", "no package here",
        "The command was given no file, so it looks for a package: a manifest in the current directory or one of its parents. There is none. Give a file, or create a manifest.";

    UNUSED_VARIABLE = "W0001", "unused variable",
        "The variable is declared but its value is never read. Remove it, or start its name with an underscore to show that this is intended.";
    UNREACHABLE = "W0002", "unreachable code",
        "The code can never run, because the code before it always returns, leaves a loop, or stops the program.";
}

impl CodeInfo {
    /// What `explain` prints: the code as the language shows it (`shown`), the title, and the
    /// explanation in lines of at most 80 columns.
    pub fn describe(&self, shown: &str) -> String {
        let mut out = format!("{shown}: {}\n\n", self.title);
        let mut column = 0;
        for word in self.explanation.split_whitespace() {
            if column > 0 && column + 1 + word.len() > 80 {
                out.push('\n');
                column = 0;
            } else if column > 0 {
                out.push(' ');
                column += 1;
            }
            out.push_str(word);
            column += word.len();
        }
        out.push('\n');
        out
    }
}

/// The meaning of a code, if it is one of A0's.
pub fn explain(code: &str) -> Option<&'static CodeInfo> {
    ALL.iter().find(|c| c.code == code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_unique_well_formed_and_explained() {
        let mut seen = std::collections::HashSet::new();
        for c in ALL {
            assert!(seen.insert(c.code), "{} twice", c.code);
            let (letter, digits) = c.code.split_at(1);
            assert!(
                matches!(letter, "E" | "W") && digits.len() == 4,
                "{}",
                c.code
            );
            assert!(digits.bytes().all(|b| b.is_ascii_digit()), "{}", c.code);
            assert!(
                !c.title.is_empty() && c.explanation.ends_with('.'),
                "{}",
                c.code
            );
        }
        assert_eq!(explain("E2001").unwrap().title, "mismatched types");
        assert!(explain("E9999").is_none());
    }

    #[test]
    fn the_documentation_lists_every_code() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/diagnostics.md");
        let docs = std::fs::read_to_string(path).unwrap();
        let rows: Vec<String> = docs
            .lines()
            .filter(|l| {
                // Rows of the table of codes: `| E2001 | ...` or `| W0001 | ...`.
                let code = l.get(2..7).unwrap_or("");
                l.starts_with("| ")
                    && code.starts_with(['E', 'W'])
                    && code[1..].bytes().all(|b| b.is_ascii_digit())
            })
            .map(str::to_string)
            .collect();
        let want: Vec<String> = ALL
            .iter()
            .map(|c| format!("| {} | {} |", c.code, c.title))
            .collect();
        assert_eq!(rows, want, "docs/diagnostics.md's table of codes");
    }
}
