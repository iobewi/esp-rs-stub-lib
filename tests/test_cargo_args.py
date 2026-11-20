import unittest

from tools.cargo_args import CargoArgsBuilder, miri_test_command


class CargoArgsBuilderTests(unittest.TestCase):
    def test_compound_subcommand_features_and_harness_order(self):
        command = (
            CargoArgsBuilder("nightly")
            .subcommand("miri")
            .subcommand_arg("test")
            .features(["feature-a", "feature-b"])
            .arg("-p")
            .arg("stub-core")
            .harness_arg("--nocapture")
            .build()
        )
        self.assertEqual(
            command,
            [
                "cargo",
                "+nightly",
                "miri",
                "test",
                "--features",
                "feature-a,feature-b",
                "-p",
                "stub-core",
                "--",
                "--nocapture",
            ],
        )

    def test_single_word_subcommand_stays_compatible(self):
        self.assertEqual(
            CargoArgsBuilder()
            .subcommand("test")
            .arg("--workspace")
            .build(),
            ["cargo", "test", "--workspace"],
        )

    def test_multiple_subcommand_arguments_stay_in_order(self):
        self.assertEqual(
            CargoArgsBuilder()
            .subcommand("miri")
            .subcommand_arg("test")
            .subcommand_arg("--doc")
            .arg("--verbose")
            .build(),
            ["cargo", "miri", "test", "--doc", "--verbose"],
        )

    def test_actual_miri_job_does_not_use_manual_features(self):
        self.assertEqual(
            miri_test_command(),
            [
                "cargo", "+nightly", "miri", "test",
                "-p", "stub-core", "-p", "stub-rom",
            ],
        )

    def test_empty_features_adds_no_flag(self):
        self.assertEqual(
            CargoArgsBuilder().subcommand("miri").subcommand_arg("test")
            .features([]).build(),
            ["cargo", "miri", "test"],
        )

    def test_separator_is_reserved_for_harness_args(self):
        with self.assertRaises(ValueError):
            CargoArgsBuilder().subcommand("--")
        with self.assertRaises(ValueError):
            CargoArgsBuilder().subcommand("miri").subcommand_arg("--")
        with self.assertRaises(ValueError):
            CargoArgsBuilder().subcommand("miri").subcommand_arg("test").arg("--")

        command = (
            CargoArgsBuilder()
            .subcommand("miri")
            .subcommand_arg("test")
            .features(["feature-a"])
            .harness_arg("--nocapture")
            .build()
        )
        self.assertEqual(
            command,
            ["cargo", "miri", "test", "--features", "feature-a", "--", "--nocapture"],
        )

    def test_argument_boundaries_cannot_be_ambiguous(self):
        with self.assertRaises(ValueError):
            CargoArgsBuilder().subcommand("miri test")
        with self.assertRaises(ValueError):
            CargoArgsBuilder().subcommand_arg("test")
        with self.assertRaises(ValueError):
            CargoArgsBuilder().subcommand("miri").subcommand_arg("test --lib")
        with self.assertRaises(ValueError):
            CargoArgsBuilder().build()


if __name__ == "__main__":
    unittest.main()
