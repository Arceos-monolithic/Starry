# Starry

NO LONGER MAINTAINED. You can check for the latest stable maintenance version in https://github.com/Starry-OS/StarryOS.

## Structure

![avatar](./doc/figures/Starry.svg)

## Build and run

```sh
# Run in unikernel architecture

# $ make A=apps/<app_name> ARCH=<arch> run

# The <app_name> is the application stored in the ./apps folder.

# The <arch> can be x86_64, risc64 and aarch64.

$ make A=apps/helloworld ARCH=x86_64 run

# Run in monolithic architecture

# Make the testcases image first

# $ ./build_img.sh <arch>

$ ./build_img.sh -m x86_64

$ make A=apps/monolithic_userboot ARCH=x86_64 run
```

## Build and run testcases with ext4fs
```sh
# Run in the lwext4fs with Rust interface, whose url is https://github.com/elliott10/lwext4_rust.
make A=apps/monolithic_userboot APP_FEATURES=batch FEATURES="lwext4" LOG=off ACCEL=n run

# Run in a new ext4fs written in Rust, whose url is https://github.com/yuoo655/ext4_rs.
make A=apps/monolithic_userboot APP_FEATURES=batch FEATURES="ext4_rs" LOG=off ACCEL=n run
```

## Pull crates to local workspace

```sh
# To download the tool
$ cargo install kbuild

$ mkdir crates

# Load crates
$ kbuild patch add linux_syscall_api

$ kbuild patch add axstarry

# Then crates will be downloaded to the crates/ folder

# To remove the crates
$ kbuild patch remove linux_syscall_api

$ kbuild patch remove axstarry

# Please verify that crates don't have any uncommitted changes before removing them.

```

## Notes

- Please remove unnecessary dependencies in `Cargo.toml` before your commit.
- After pulling a new crate to the local workspace, maybe you need to execute `make clean` to update the cache.

