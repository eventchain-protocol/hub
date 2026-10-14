/* The GHC runtime must be started before any exported Haskell function runs
 * and stopped after the last one. Wrapping hs_init/hs_exit here means the
 * Rust side links one library and declares two plain C functions. */
#include <HsFFI.h>

void ec_init(void)
{
    int argc = 1;
    char *argv[] = {"eventchain_acceptor", 0};
    char **pargv = argv;
    hs_init(&argc, &pargv);
}

void ec_exit(void)
{
    hs_exit();
}
