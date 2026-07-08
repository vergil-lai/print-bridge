import { spawnSync } from 'node:child_process';
import { existsSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

const root = process.cwd();

function run(command, args, cwd = root, env = {}) {
  const result = spawnSync(command, args, {
    cwd,
    env: { ...process.env, ...env },
    stdio: 'inherit',
  });
  if (result.status !== 0) {
    process.exit(result.status ?? 1);
  }
}

run('php', ['examples/config-transfer/php/config_transfer.php', 'verify']);
run('go', ['run', '.', 'verify'], join(root, 'examples/config-transfer/go'), {
  GOCACHE: join(tmpdir(), 'printbridge-go-build-cache'),
});

const nodeExample = join(root, 'examples/config-transfer/node');
if (!existsSync(join(nodeExample, 'node_modules'))) {
  run('npm', ['install'], nodeExample);
}
run('npm', ['test'], nodeExample);
