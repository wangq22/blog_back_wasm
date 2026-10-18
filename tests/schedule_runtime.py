"""Optional end-to-end smoke test against an ISOLATED local Wrangler Worker.
Never point this at production: creates/deletes synthetic tasks.
CALENDAR_TEST_URL defaults to localhost; CALENDAR_TEST_TOKEN is a local fixture token.
"""
import concurrent.futures
import datetime as dt
import json
import os
import urllib.parse
import urllib.request
import urllib.error

BASE = os.environ.get('CALENDAR_TEST_URL', 'http://127.0.0.1:8787')
assert urllib.parse.urlsplit(BASE).hostname in ('127.0.0.1', 'localhost'), 'Local fixtures only'
TOKEN = os.environ.get('CALENDAR_TEST_TOKEN', 'calendar-local-test-only')


def rpc(method, params):
    data = json.dumps(dict(jsonrpc='2.0', id=1, method=method, params=params)).encode()
    request = urllib.request.Request(BASE + '/mcp', data, headers={'Authorization': 'Bearer ' + TOKEN, 'Content-Type': 'application/json', 'Accept': 'application/json, text/event-stream'})
    with urllib.request.urlopen(request, timeout=30) as response:
        result = json.load(response)
    assert 'error' not in result, result
    return result['result']


def tool(name, arguments=None, error=False):
    result = rpc('tools/call', dict(name=name, arguments=arguments or {}))
    assert result.get('isError', False) == error, result
    return result['structuredContent']


def iso(value):
    return value.isoformat(timespec='seconds').replace('+00:00', 'Z')


def run():
    definitions = rpc('tools/list', {})['tools']
    names = [t['name'] for t in definitions]
    assert {'replan_schedule', 'update_schedule_settings', 'lock_schedule_block'} <= set(names)
    for definition in definitions:
        if definition['name'] in ('create_task', 'update_task'):
            assert definition['inputSchema']['properties']['priority_level']['enum'] == ['P1', 'P2', 'P3', 'P4', 'P5']
            assert 'depends_on' in definition['inputSchema']['properties']
    before = tool('list_tasks')
    assert not before['tasks'], 'Use an empty isolated test database'
    now = dt.datetime.now(dt.timezone.utc).replace(microsecond=0)
    created = tool('create_task', dict(title='Integration flex', notes='PRIVATE FIXTURE NOTE', estimated_minutes=210, difficulty=5, priority=5, splittable=True, min_chunk_minutes=30, max_chunk_minutes=90))['task']
    assert sum(b['estimated_minutes'] for b in created['blocks']) == 210, created
    assert all(30 <= b['estimated_minutes'] <= 90 for b in created['blocks'])
    assert created['assessment']['source'] == 'fallback'
    version = created['assessment']['version']
    updated = tool('update_task', dict(id=created['id'], title=created['title'], notes='PRIVATE FIXTURE NOTE', priority=4, replan=True))['task']
    assert updated['priority'] == 4 and updated['assessment']['version'] == version
    pinned = updated['blocks'][-1]
    tool('lock_schedule_block', dict(id=pinned['id'], locked=True))
    task = next(t for t in tool('list_tasks')['tasks'] if t['id'] == created['id'])
    assert any(b['id'] == pinned['id'] and b['locked'] for b in task['blocks'])
    start = now + dt.timedelta(days=2, minutes=7, seconds=11)
    end = start + dt.timedelta(minutes=123)
    fixed = tool('create_task', dict(title='Integration contest', mode='fixed', fixed_start=iso(start), fixed_end=iso(end)))['task']
    assert fixed['blocks'][0]['scheduled_start'] == iso(start) and fixed['blocks'][0]['scheduled_end'] == iso(end)
    overlap = tool('create_task', dict(title='Integration overlap', mode='fixed', fixed_start=iso(start + dt.timedelta(minutes=30)), fixed_end=iso(end)))['task']
    assert overlap['schedule_state'] == 'conflict'
    waiting = tool('create_task', dict(title='Integration past deadline', deadline=iso(now - dt.timedelta(minutes=5)), estimated_minutes=60))['task']
    assert waiting['schedule_state'] == 'unscheduled' and not waiting['blocks']
    design = tool('create_task', dict(title='Integration design', priority_level='P5', estimated_minutes=105))['task']
    implement = tool('create_task', dict(title='Integration implementation', priority_level='P1', depends_on=[design['id']], estimated_minutes=60))['task']
    assert implement['priority'] == 5 and implement['priority_level'] == 'P1'
    assert implement['depends_on'] == [design['id']]
    tasks = {t['id']:t for t in tool('list_tasks')['tasks']}
    assert max(b['scheduled_end'] for b in tasks[design['id']]['blocks']) <= min(b['scheduled_start'] for b in tasks[implement['id']]['blocks'])
    saved_revision = tool('list_tasks')['revision']
    tool('update_task', dict(id=design['id'], title=design['title'], depends_on=[implement['id']]), error=True)
    tool('update_task', dict(id=design['id'], title=design['title'], depends_on=[design['id']]), error=True)
    tool('update_task', dict(id=design['id'], title=design['title'], depends_on=[999999]), error=True)
    tool('delete_task', dict(id=design['id']), error=True)
    tool('update_task', dict(id=implement['id'], title=implement['title'], priority_level='P1', priority=1), error=True)
    tool('create_task', dict(title='Invalid priority', priority_level='P0'), error=True)
    assert tool('list_tasks')['revision'] == saved_revision, 'Invalid graph/priority edits must be atomic'
    implement = tool('update_task', dict(id=implement['id'], title=implement['title'], priority_level='P2', depends_on=[]))['task']
    assert implement['priority'] == 4 and implement['priority_level'] == 'P2' and implement['depends_on'] == []
    implement = tool('update_task', dict(id=implement['id'], title=implement['title'], notes='Notes edit keeps explicit priority', depends_on=[design['id']]))['task']
    assert implement['priority_level'] == 'P2'
    blocked = tool('create_task', dict(title='Integration dependent waiting', depends_on=[waiting['id']], estimated_minutes=30))['task']
    assert blocked['schedule_state'] == 'unscheduled' and not blocked['blocks']
    assert blocked['unscheduled_reason'].startswith('dependency_blocked:')
    settings = tool('get_schedule_settings')
    settings['settings']['energy_weekday'][0] = 71
    tool('update_schedule_settings', dict(settings=settings['settings'], expected_revision=settings['revision']))
    tool('update_schedule_settings', dict(settings=settings['settings'], expected_revision=settings['revision']), error=True)
    with urllib.request.urlopen(BASE + '/api/schedule/tasks') as response:
        public = json.load(response)
    assert 'PRIVATE FIXTURE NOTE' not in json.dumps(public)
    assert all('assessment' not in t and 'notes' not in t and 'depends_on' not in t for t in public)
    assert waiting['id'] not in [t['id'] for t in public]
    try:
        urllib.request.urlopen(urllib.request.Request(BASE + f"/api/schedule/tasks/{waiting['id']}/reactions", json.dumps({'emoji':'👏'}).encode(), headers={'Content-Type':'application/json'}))
        raise AssertionError('Unscheduled private tasks must not accept public reactions')
    except urllib.error.HTTPError as error:
        assert error.code == 404

    with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:
        parallel = list(pool.map(lambda n: tool('create_task', dict(title=f'Integration concurrent {n}', estimated_minutes=30)), [1, 2]))
    assert parallel[0]['task']['id'] != parallel[1]['task']['id']
    past = tool('create_task', dict(title='Integration elapsed event', mode='fixed', fixed_start=iso(now-dt.timedelta(days=2, hours=1)), fixed_end=iso(now-dt.timedelta(days=2))))['task']
    with urllib.request.urlopen(BASE + '/__scheduled?cron=' + urllib.parse.quote('*/5 * * * *'), timeout=30) as response:
        response.read()
    reviews = tool('list_reviews')
    review = next(r for r in reviews['reviews'] if r['task_id'] == past['id'])
    tool('save_review', dict(review_id=review['id'], block_id=review['block_id'], outcome='completed', task_completed=True, spent_minutes=55, energy_rating=4, expected_revision=reviews['revision']))
    tool('save_review', dict(review_id=review['id'], outcome='completed'), error=True)
    assert next(t for t in tool('list_tasks')['tasks'] if t['id'] == past['id'])['status'] == 'done'
    experience = tool('get_scheduling_experience')
    assert experience['history'], 'Manual profile updates must have an audit trail'
    owned = {created['id'], fixed['id'], overlap['id'], waiting['id'], past['id'], design['id'], implement['id'], blocked['id'], *(entry['task']['id'] for entry in parallel)}
    for task in sorted(tool('list_tasks')['tasks'], key=lambda t:t['id'], reverse=True):
        if task['id'] in owned:
            tool('delete_task', dict(id=task['id']))
    assert not tool('list_tasks')['tasks']
    print('PASS: real WASM/D1/MCP priorities P1/P2, dependencies, graph validation/atomicity, split, update, pin, fixed conflicts, waiting list, settings CAS, privacy, concurrent creates, cron, reviews, audit, delete')


def isolated():
    import pathlib
    import socket
    import subprocess
    import tempfile
    import time
    global BASE
    root = pathlib.Path(__file__).resolve().parents[1]
    wrangler = root / 'node_modules/wrangler/bin/wrangler.js'
    main = root / 'build/index.js'
    assert main.exists(), 'Run worker-build --release first'
    with socket.socket() as sock:
        sock.bind(('127.0.0.1', 0))
        port = sock.getsockname()[1]
    BASE = f'http://127.0.0.1:{port}'
    with tempfile.TemporaryDirectory(prefix='calendar-integration-') as directory:
        path = pathlib.Path(directory)
        config = path / 'wrangler.toml'
        config.write_text(f'''name = "calendar-integration"
main = {json.dumps(str(main))}
compatibility_date = "2025-10-09"
[vars]
CF_ACCESS_TEAM_DOMAIN = "https://local-test.cloudflareaccess.com"
CF_ACCESS_CLIENT_ID = "local-test"
MCP_BEARER_TOKEN = {json.dumps(TOKEN)}
MCP_ISSUER = {json.dumps(BASE)}
MCP_RESOURCE_URL = {json.dumps(BASE + '/mcp')}
[[d1_databases]]
binding = "DB"
database_name = "calendar-integration"
database_id = "00000000-0000-0000-0000-000000000001"
''')
        env = dict(os.environ, WRANGLER_LOG_PATH=str(path / 'wrangler.log'), WRANGLER_SEND_METRICS='false')
        for migration in ['0005_schedule.sql', '0006_schedule_task_management.sql', '0009_deterministic_schedule.sql']:
            result = subprocess.run(['node', str(wrangler), 'd1', 'execute', 'calendar-integration', '--local', '--config', str(config), '--file', str(root / 'migrations' / migration)], cwd=path, env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
            assert result.returncode == 0, result.stdout
        with (path / 'runtime.log').open('w+') as log:
            server = subprocess.Popen(['node', str(wrangler), 'dev', '--config', str(config), '--local', '--port', str(port), '--test-scheduled', '--log-level', 'error'], cwd=path, env=env, stdout=log, stderr=subprocess.STDOUT)
            try:
                for _ in range(100):
                    if server.poll() is not None:
                        log.seek(0)
                        raise RuntimeError(log.read())
                    try:
                        with urllib.request.urlopen(BASE + '/api/schedule/config', timeout=1):
                            break
                    except OSError:
                        time.sleep(0.1)
                else:
                    raise RuntimeError('Local Worker did not become ready')
                run()
            finally:
                server.terminate()
                try:
                    server.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    server.kill()
                    server.wait()


if __name__ == '__main__':
    import sys
    isolated() if '--isolated' in sys.argv else run()
