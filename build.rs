//! Link the Haskell acceptor library built by `cabal build` in `acceptor/`.
//! The library's own dependencies (GHC runtime, libcrypto) are resolved by
//! the install names cabal wrote into it, so only its directory is needed.

fn main() {
    let pattern = "acceptor/dist-newstyle/build/*/ghc-*/eventchain-acceptor-*/f/eventchain_acceptor/build/eventchain_acceptor/libeventchain_acceptor.*";
    let found: Vec<_> = glob::glob(pattern).expect("valid glob").filter_map(Result::ok).collect();
    let lib = match found.as_slice() {
        [one] => one,
        [] => panic!("acceptor library not built: run `cabal build` in acceptor/ first"),
        many => panic!("more than one acceptor library built: {many:?}"),
    };
    let dir = std::fs::canonicalize(lib.parent().expect("library has a directory")).expect("canonical path");
    println!("cargo:rerun-if-changed={}", lib.display());
    println!("cargo:rustc-link-search=native={}", dir.display());
    println!("cargo:rustc-link-lib=dylib=eventchain_acceptor");
    println!("cargo:rustc-link-arg=-Wl,-rpath,{}", dir.display());
}
