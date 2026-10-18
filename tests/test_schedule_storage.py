"""Exercise the actual bulk-write SQL against SQLite (the D1 SQL dialect).
Run: python3 -m unittest discover -s tests -p 'test_schedule_storage.py'
"""
import json
import pathlib
import re
import sqlite3
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[1]
SOURCE = (ROOT / 'src/route/schedule.rs').read_text()
GUARD = '(SELECT commit_token FROM schedule_state WHERE _id=1)=?2'
QUERIES = re.findall(r'let\s+query\s*=\s*format!\(\s*"([^"]+)"', SOURCE)
QUERIES = [q.replace('{GUARD}', GUARD) for q in QUERIES]


class CalendarStorage(unittest.TestCase):
    def setUp(self):
        self.db = sqlite3.connect(':memory:')
        self.db.execute('PRAGMA foreign_keys=ON')
        for name in ['0005_schedule.sql', '0006_schedule_task_management.sql']:
            self.db.executescript((ROOT / 'migrations' / name).read_text())
        self.db.execute("INSERT INTO schedule_tasks (_id,title,scheduled_start,scheduled_end,estimated_minutes,status,created_at,updated_at) VALUES (1,'Legacy task','2026-10-19T12:00:00Z','2026-10-19T13:00:00Z',60,'awaiting_review','','')")
        self.db.execute("INSERT INTO schedule_task_reviews (task_id,status,prompted_at) VALUES (1,'pending','')")
        self.db.execute("INSERT INTO schedule_learning (_id,content,ai_model,updated_at) VALUES (1,'Keep the old reflection memory','legacy-model','2026-10-18T12:00:00Z')")
        self.db.commit()
        self.db.executescript((ROOT / 'migrations/0009_deterministic_schedule.sql').read_text())
        self.task = dict(id=2, title='New task', notes='private', deadline='', status='planned', planning_json='{}', start='2026-10-20T12:00:00Z', end='2026-10-20T13:00:00Z', minutes=60, reason='deterministic', model='', actual_minutes=None, summary='', created_at='', updated_at='')
        self.block = dict(id='2:1:0', task_id=2, start=self.task['start'], end=self.task['end'], status='planned', data_json='{}')
        self.review = dict(id=None, task_id=2, block_id=self.block['id'], status='pending', outcome='', prompted_at='', reviewed_at='', spent_minutes=None, summary='', data_json='{}')
        self.assertEqual(len(QUERIES), 3, 'tests execute the three production bulk-write statements')

    def write(self, expected, token, task=None):
        changed = self.db.execute('UPDATE schedule_state SET revision=revision+1,commit_token=?1,data_json=?2 WHERE _id=1 AND revision=?3 RETURNING revision', (token, '{}', expected)).fetchall()
        for query, rows in zip(QUERIES, [[task or self.task], [self.block], [self.review]]):
            self.db.execute(query, (json.dumps(rows), token))
        return changed

    def test_legacy_snapshot_and_pending_review_survive(self):
        raw = self.db.execute('SELECT data_json FROM schedule_blocks').fetchone()[0]
        self.assertTrue(json.loads(raw)['legacy'])
        state = json.loads(self.db.execute('SELECT data_json FROM schedule_state').fetchone()[0])
        self.assertEqual(state['learning']['content'], 'Keep the old reflection memory')
        self.assertEqual(self.db.execute('SELECT block_id FROM schedule_task_reviews').fetchone()[0], 'legacy:1')

    def test_atomic_writes_stale_guard_and_review_idempotence(self):
        with self.db:
            self.assertEqual(self.write(0, 'first'), [(1,)])
        self.assertEqual(self.db.execute('SELECT count(*) FROM schedule_tasks').fetchone()[0], 2)
        with self.db:
            altered = dict(self.task, title='STALE OVERWRITE')
            self.assertEqual(self.write(0, 'stale', altered), [])
            self.db.execute(f'DELETE FROM schedule_tasks WHERE _id IN (SELECT value FROM json_each(?1)) AND {GUARD}', ('[2]', 'stale'))
        self.assertEqual(self.db.execute('SELECT title FROM schedule_tasks WHERE _id=2').fetchone()[0], 'New task')
        with self.db:
            self.write(1, 'second')
        self.assertEqual(self.db.execute("SELECT count(*) FROM schedule_task_reviews WHERE block_id='2:1:0'").fetchone()[0], 1)

    def test_constraint_failure_rolls_back_revision_and_whole_batch(self):
        with self.db:
            self.write(0, 'good')
        with self.assertRaises(sqlite3.IntegrityError):
            with self.db:
                self.write(1, 'bad', dict(self.task, title=None))
        self.assertEqual(self.db.execute('SELECT revision,commit_token FROM schedule_state').fetchone(), (1, 'good'))
        self.assertEqual(self.db.execute('SELECT title FROM schedule_tasks WHERE _id=2').fetchone()[0], 'New task')

    def test_mid_batch_foreign_key_failure_rolls_back_prior_task_insert(self):
        self.block['task_id'] = 999
        with self.assertRaises(sqlite3.IntegrityError):
            with self.db:
                self.write(0, 'broken-block')
        self.assertEqual(self.db.execute('SELECT revision FROM schedule_state').fetchone()[0], 0)
        self.assertIsNone(self.db.execute('SELECT _id FROM schedule_tasks WHERE _id=2').fetchone())


if __name__ == '__main__':
    unittest.main()
