#!/usr/bin/env node
import { readFileSync } from 'node:fs'
import { isAbsolute, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

export function npmWorkspacePaths(manifest) {
  if (!Array.isArray(manifest?.workspaces) || manifest.workspaces.length === 0) {
    throw new Error('package.json workspaces must be a non-empty array of static relative paths')
  }
  const paths = new Set()
  for (const [index, workspace] of manifest.workspaces.entries()) {
    if (typeof workspace !== 'string' || workspace.split('/').some(segment => !segment
      || segment === '.' || segment === '..' || /[^A-Za-z0-9_.-]/.test(segment))) {
      throw new Error(`Invalid npm workspace path at index ${index}: use static relative paths with A-Za-z0-9_.- segments separated by /; globs and traversal are not supported`)
    }
    paths.add(workspace)
  }
  return [...paths]
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    const args = process.argv.slice(2)
    if (args.length !== 1 || !isAbsolute(args[0])) {
      throw new Error('Usage: node npm-workspace-dependencies.mjs /absolute/path/to/package.json')
    }
    const paths = npmWorkspacePaths(JSON.parse(readFileSync(args[0], 'utf8')))
    process.stdout.write(`${paths.join('\n')}\n`)
  } catch (error) {
    process.stderr.write(`${error.message}\n`)
    process.exitCode = 1
  }
}
