import json
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


class ProgressTest(unittest.TestCase):
    def test_unique_ids_and_ranges(self):
        data = json.loads((ROOT / 'docs/progress.json').read_text())
        seen = set()
        for milestone in data['milestones']:
            for phase in milestone['phases']:
                for task in phase['tasks']:
                    self.assertNotIn(task['id'], seen)
                    seen.add(task['id'])
                    self.assertTrue(0 <= task['progress'] <= 100)
            for feature in milestone['features']:
                self.assertTrue(0 <= feature['progress'] <= 100)


if __name__ == '__main__':
    unittest.main()
