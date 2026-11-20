"""Build Cargo command arguments with compound subcommands.

Inspired by the review requirement in esp-rs/esp-hal#6856:
subcommand("miri").subcommand_arg("test") must emit both words
before options such as --features. No shell splitting or manual feature
injection is needed.
"""
from __future__ import annotations

import subprocess
import sys


class CargoArgsBuilder:
    def __init__(self, toolchain: str | None = None) -> None:
        self._toolchain = toolchain
        self._subcommand: list[str] = []
        self._features: list[str] = []
        self._args: list[str] = []
        self._harness_args: list[str] = []

    def subcommand(self, command: str) -> "CargoArgsBuilder":
        if not command or command == "--" or any(c.isspace() for c in command):
            raise ValueError("subcommand must be one nonempty word")
        self._subcommand = [command]
        return self

    def subcommand_arg(self, argument: str) -> "CargoArgsBuilder":
        if not self._subcommand:
            raise ValueError("subcommand() must be called first")
        if not argument or argument == "--" or any(c.isspace() for c in argument):
            raise ValueError("subcommand argument must be one nonempty word")
        self._subcommand.append(argument)
        return self

    def features(self, features: list[str]) -> "CargoArgsBuilder":
        self._features.extend(features)
        return self

    def arg(self, argument: str) -> "CargoArgsBuilder":
        if argument == "--":
            raise ValueError("use harness_arg() for test-harness arguments")
        self._args.append(argument)
        return self

    def harness_arg(self, argument: str) -> "CargoArgsBuilder":
        self._harness_args.append(argument)
        return self

    def build(self) -> list[str]:
        if not self._subcommand:
            raise ValueError("missing subcommand")
        args = ["cargo"]
        if self._toolchain:
            args.append(f"+{self._toolchain}")
        args.extend(self._subcommand)
        if self._features:
            args.extend(["--features", ",".join(self._features)])
        args.extend(self._args)
        if self._harness_args:
            args.append("--")
            args.extend(self._harness_args)
        return args


def miri_test_command() -> list[str]:
    return (
        CargoArgsBuilder("nightly")
        .subcommand("miri")
        .subcommand_arg("test")
        .arg("-p")
        .arg("stub-core")
        .arg("-p")
        .arg("stub-rom")
        .build()
    )


if __name__ == "__main__":
    # Subprocess uses argv directly; no shell interpretation.
    raise SystemExit(subprocess.call(miri_test_command()))
