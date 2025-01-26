#!/usr/bin/env bash

set -e

declare -a BINS=(
  antegen
)

usage() {
  exitcode=0
  if [[ -n "$1" ]]; then
    exitcode=1
    echo "Error: $*"
  fi
  cat <<EOF
usage: $0 [--release] [--target <target triple>] <install directory>
EOF
  exit $exitcode
}

# Set default target triple from 'cargo -vV'
defaultTargetTriple=$(cargo -vV | grep 'host:' | cut -d ' ' -f2)

# Set build flags
installDir=
buildVariant=debug
maybeReleaseFlag=
targetTriple="$defaultTargetTriple"
while [[ -n $1 ]]; do
  if [[ ${1:0:1} = - ]]; then
    case $1 in
      --release)
        maybeReleaseFlag=--release
        buildVariant=release
        shift
        ;;
      --target)
        targetTriple=$2
        shift 2
        ;;
      *)
        usage "Unknown option: $1"
        ;;
    esac
  else
    installDir=$1
    shift
  fi
done

# If target triple is still unset, use default
if [[ -z "$targetTriple" ]]; then
  targetTriple="$defaultTargetTriple"
fi

# Print final configuration
echo "Build variant: $buildVariant"
echo "Target triple: $targetTriple"
echo "Install directory: $installDir"
echo "Release flag: ${maybeReleaseFlag:---not-set}"

# Check the install directory is provided
if [[ -z "$installDir" ]]; then
  usage "Install directory not specified"
  exit 1
fi

# Create the install directory
installDir="$(mkdir -p "$installDir"; cd "$installDir"; pwd)"
mkdir -p "$installDir/lib"
mkdir -p "$installDir/bin"
echo "Install location: $installDir ($buildVariant)"
cd "$(dirname "$0")"/..
SECONDS=0

# Build programs
(
  set -x
  source ~/.bash_profile
  anchor build
)

# Define lib extension
case $targetTriple in
  *darwin*)
    pluginFilename=libantegen_plugin.dylib
    ;;
  *)
    pluginFilename=libantegen_plugin.so
    ;;
esac

# Build the repo
(
  set -x
  cargo build --workspace --locked $maybeReleaseFlag --target "$targetTriple"
  
  # Copy binaries
  case $targetTriple in
    *darwin*)
      pluginFilename=libantegen_plugin.dylib
      cp -fv "target/$targetTriple/$buildVariant/$pluginFilename" "$installDir"/lib
      mv "$installDir"/lib/libantegen_plugin.dylib "$installDir"/lib/libantegen_plugin.so
      ;;
    *)
      pluginFilename=libantegen_plugin.so
      cp -fv "target/$targetTriple/$buildVariant/$pluginFilename" "$installDir"/lib
      ;;
  esac

  for bin in "${BINS[@]}"; do
    rm -fv "$installDir/bin/$bin"
    cp -fv "target/$targetTriple/$buildVariant/$bin" "$installDir/bin"
  done

  cp -fv "target/deploy/antegen_network_program.so" "$installDir/lib"
  cp -fv "target/deploy/antegen_thread_program.so" "$installDir/lib"
)

# Success message
echo "Done after $SECONDS seconds"
echo 
echo "To use these binaries:"
echo "  export PATH=$installDir/bin:\$PATH"

