"""Dependency-free route contract regression; real Caddy probe is separate."""
import pathlib
import re
import unittest


class ReadyRoute(unittest.TestCase):
    def test_ready_has_explicit_api_handler_before_spa(self):
        config = (pathlib.Path(__file__).parents[1] / "Caddyfile").read_text()
        config = re.sub(r"#.*", "", config)
        handler = re.search(r"handle\s+/ready\s*\{([^}]+)\}", config)
        self.assertIsNotNone(handler, "/ready falls through to the SPA instead of API readiness")
        self.assertRegex(handler.group(1), r"reverse_proxy\s+api:3001")
        self.assertLess(handler.start(), config.index("try_files"))


if __name__ == "__main__":
    unittest.main(verbosity=2)
