import { spawnSync } from 'node:child_process'
import { createServer } from 'node:net'

const runCommand = (command, args, options) => spawnSync(command, args, {
  encoding: 'utf8', windowsHide: true, ...options,
})

export function githubRepository(remote) {
  const value = String(remote).trim()
  const ssh = value.match(/^git@([a-z0-9.-]+):([\w.-]+)\/([\w.-]+?)(?:\.git)?$/i)
  const url = ssh ? null : (() => { try { return new URL(value) } catch { return null } })()
  if (ssh) return { host: ssh[1].toLowerCase(), owner: ssh[2], repository: ssh[3], protocol: 'ssh' }
  if (!url || !['https:', 'ssh:'].includes(url.protocol) || url.password || url.search || url.hash
    || (url.username && url.username !== 'git') || url.port) throw new Error('origin 必须是无内嵌凭据的 GitHub HTTPS 或 SSH 地址')
  const path = url.pathname.match(/^\/([\w.-]+)\/([\w.-]+?)(?:\.git)?\/?$/)
  if (!path) throw new Error('无法从 origin 确定 GitHub 仓库')
  return { host: url.hostname.toLowerCase(), owner: path[1], repository: path[2], protocol: url.protocol === 'ssh:' ? 'ssh' : 'https' }
}

export function githubEnvironment(environment = process.env) {
  // Diagnostics must never dump authorization headers; background probes must never open login UI.
  const result = { ...environment, GH_PROMPT_DISABLED: '1', GH_DEBUG: '', DEBUG: '', GH_PAGER: 'cat',
    GIT_TERMINAL_PROMPT: '0', GCM_INTERACTIVE: 'Never', GIT_TRACE: '0', GIT_TRACE_CURL: '0', GIT_CURL_VERBOSE: '0' }
  const ssh = environment.GIT_SSH_COMMAND || (environment.GIT_SSH ? `'${environment.GIT_SSH.replaceAll("'", "'\\''")}'` : 'ssh')
  const batch = /plink/i.test(environment.GIT_SSH_VARIANT || ssh) ? '-batch' : '-o BatchMode=yes -o StrictHostKeyChecking=yes'
  result.GIT_SSH_COMMAND = ssh.includes(batch) ? ssh : `${ssh} ${batch}`
  return result
}

export function readGitHubAuth(gh, host, { run = runCommand, cwd, environment = process.env } = {}) {
  const result = run(gh, ['auth', 'status', '--hostname', host, '--active', '--json', 'hosts'], {
    cwd, env: githubEnvironment(environment), timeout: 30000,
  })
  if (result.error || result.status !== 0) return { ok: false, reason: 'GitHub 认证检查失败，请检查网络、gh 版本或登录状态' }
  let entries
  try { entries = JSON.parse(result.stdout).hosts?.[host] } catch { /* Report a controlled message, never raw auth output. */ }
  const account = Array.isArray(entries) ? entries.find(entry => entry.active) : null
  if (!account || account.state !== 'success') return { ok: false, reason: '没有有效的 GitHub 登录，或认证服务暂时不可达' }
  const source = account.tokenSource
  if (!['keyring', 'GH_TOKEN', 'GITHUB_TOKEN', 'GH_ENTERPRISE_TOKEN', 'GITHUB_ENTERPRISE_TOKEN'].includes(source)) {
    return { ok: false, storageError: true, reason: 'GitHub 凭据未保存在系统凭据管理器中。请修复凭据管理器并重新登录；setup 不接受明文凭据文件作为认证完成状态' }
  }
  if (!/^[\w-]+$/.test(account.login)) return { ok: false, reason: 'GitHub 返回的账号信息无效' }
  return { ok: true, login: account.login, source }
}

export async function prepareGitHubAuthentication({
  gh, git, cwd, assumeYes = false, ask, log = console.log, environment = process.env,
  run = runCommand, login = runCommand,
}) {
  if (!gh || !git) throw new Error('Git 和 GitHub CLI 必须先安装完成；尚未开始其余工具的安装')
  const env = githubEnvironment(environment)
  const options = { cwd, env, timeout: 30000 }
  const remote = run(git, ['remote', 'get-url', 'origin'], options)
  if (remote.error || remote.status !== 0) throw new Error('无法读取 origin，请先配置当前仓库的 GitHub 远程地址')
  const target = githubRepository(remote.stdout)
  if (target.protocol === 'ssh' && !environment.GIT_SSH_COMMAND && !environment.GIT_SSH) {
    const configured = run(git, ['config', '--get', 'core.sshCommand'], options)
    if (!configured.error && configured.status === 0 && String(configured.stdout).trim()) {
      env.GIT_SSH_COMMAND = githubEnvironment({ ...environment, GIT_SSH_COMMAND: String(configured.stdout).trim() }).GIT_SSH_COMMAND
    }
  }
  const tokenNames = target.host === 'github.com' ? ['GH_TOKEN', 'GITHUB_TOKEN'] : ['GH_ENTERPRISE_TOKEN', 'GITHUB_ENTERPRISE_TOKEN']
  const hasEnvironmentToken = tokenNames.some(name => Boolean(environment[name]))
  const probe = () => readGitHubAuth(gh, target.host, { run, cwd, environment })
  const browserLogin = () => {
    if (hasEnvironmentToken) throw new Error('当前环境中的 GitHub token 无效或权限不足，请先修复对应环境变量；不会覆盖它或改用其他账号')
    log(`请现在完成 ${target.host} 浏览器授权（可能需要验证码或组织授权），完成前不会开始其余工具的安装。`)
    const loginEnv = { ...env }
    delete loginEnv.GH_PROMPT_DISABLED
    const result = login(gh, ['auth', 'login', '--hostname', target.host, '--web', '--git-protocol', target.protocol, '--skip-ssh-key'], {
      cwd, env: loginEnv, stdio: 'inherit', timeout: 600000,
    })
    if (result.error || result.status !== 0) throw new Error('GitHub 浏览器授权未完成或已超时，尚未开始其余工具的安装；重新执行 setup 可复用已装工具')
  }
  for (let attempt = 0; attempt < 3; attempt += 1) {
    log(`[认证] 检查 ${target.host} 当前账号…`)
    let auth = probe()
    if (!auth.ok && !auth.storageError && !assumeYes && attempt === 0) { browserLogin(); auth = probe() }
    let failure = auth.ok ? '' : auth.reason
    if (auth.ok) {
      log(`[认证] 当前账号：${auth.login}；凭据来源：${auth.source}`)
      log(`[认证] 检查 ${target.owner}/${target.repository} 的 PR 读取权限…`)
      const access = run(gh, ['api', '--hostname', target.host, `repos/${target.owner}/${target.repository}/pulls?state=open&per_page=1`, '--jq', 'length'], options)
      if (access.error || access.status !== 0) failure = '当前账号无法读取仓库 PR，请检查仓库权限、组织 SSO 授权和网络'
      else {
        log('[认证] 检查 Git 远程读取（不弹出登录窗口）…')
        let fetch = run(git, ['ls-remote', '--exit-code', 'origin', 'refs/heads/main'], options)
        if ((fetch.error || fetch.status !== 0) && target.protocol === 'https') {
          log(`[认证] 为 ${target.host} 配置 gh Git 凭据助手…`)
          const helper = run(gh, ['auth', 'setup-git', '--hostname', target.host], options)
          if (!helper.error && helper.status === 0) fetch = run(git, ['ls-remote', '--exit-code', 'origin', 'refs/heads/main'], options)
        }
        if (fetch.error || fetch.status !== 0) failure = target.protocol === 'ssh'
          ? 'Git SSH 访问失败；请现在配置 SSH 密钥、agent 和主机信任，或手动将 origin 改为 HTTPS 后重新执行 setup'
          : 'Git 远程读取失败，请检查网络、凭据和 origin/main 是否存在'
      }
      if (!failure) {
        log(`[PASS] GitHub 账号、仓库 PR 和 Git 远程访问已验证：${auth.login}`)
        return { ...target, ...auth, gitSshCommand: env.GIT_SSH_COMMAND }
      }
    }
    if (assumeYes) throw new Error(`${failure}。--yes 要求已有可用认证；请先交互运行 setup，尚未开始其余工具的安装`)
    if (attempt === 2) throw new Error(`${failure}。前置检查未通过，尚未开始其余工具的安装`)
    log(failure)
    const answer = String(await ask('处理完成后回车重试；输入 login 重新浏览器登录；输入 q 退出：')).trim().toLowerCase()
    if (answer === 'q') throw new Error('已退出 GitHub 前置认证，尚未开始其余工具的安装')
    if (answer === 'login') browserLogin()
  }
}

export async function runNativeSetupPhases({ packages, install, authenticate, automatic, log = console.log }) {
  const bootstrap = entry => ['bash', 'gh'].includes(entry.key)
  log('\n[阶段 1/2] 准备 Git、GitHub CLI 和认证；请暂时留在电脑前。')
  for (const entry of packages.filter(bootstrap)) await install(entry)
  await authenticate()
  log('\n[阶段 2/2] 前置确认和认证已完成，现在可以离开；后续自动安装，结束时汇总结果。')
  return automatic(packages.filter(entry => !bootstrap(entry)))
}

export function acquireSetupLock(endpoint = '\\\\.\\pipe\\aster-windows-native-setup') {
  return new Promise((accept, reject) => {
    const server = createServer(socket => socket.end())
    server.once('error', error => reject(new Error(error.code === 'EADDRINUSE'
      ? '已有 setup 命令正在运行，请返回原终端查看；不会启动第二轮安装' : `无法建立 setup 互斥锁：${error.code}`)))
    server.listen(endpoint, () => {
      server.unref()
      accept({ endpoint: server.address(), release: () => new Promise(done => server.close(done)) })
    })
  })
}
