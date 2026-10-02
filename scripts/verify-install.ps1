# Check an installed kanbanr is the release it claims to be, and that it works (FEAT-149).
#
#   $env:GITHUB_REF_NAME = 'v0.1.2'; $env:GITHUB_SHA = '<commit>'; pwsh scripts/verify-install.ps1
#
# The Windows twin of scripts/verify-install.sh: run by release.yml after install.ps1 has
# installed the just-published release. Same checks, same order, same messages.
$ErrorActionPreference = 'Stop'
$tag = $env:GITHUB_REF_NAME
$commit = $env:GITHUB_SHA
if (-not $tag -or -not $commit) { throw 'GITHUB_REF_NAME and GITHUB_SHA must name the release tag and its commit' }

# It installed, and it names both the version and the commit.
$v = (& kanbanr --version | Out-String).Trim()
if ($LASTEXITCODE -ne 0) { throw "kanbanr --version failed: $v" }
Write-Host $v
if (-not $v.Contains($tag.TrimStart('v'))) {
  Write-Host "::error::--version does not name the released version: $v"; exit 1
}
$sha = $commit.Substring(0, 12)
if (-not $v.Contains($sha)) {
  Write-Host "::error::--version does not name the released commit: $v"; exit 1
}

# The monitor is served with no --ui-dir and no build step.
$board = Join-Path ([IO.Path]::GetTempPath()) ("kanbanr-verify-" + [guid]::NewGuid().ToString('N')) | Join-Path -ChildPath 'board'
$port = if ($env:KANBANR_VERIFY_PORT) { $env:KANBANR_VERIFY_PORT } else { '8080' }
& kanbanr init ci-check --data-dir $board --no-hooks --author CI --email ci@kanbanr.local
if ($LASTEXITCODE -ne 0) { throw 'kanbanr init failed' }
$server = Start-Process kanbanr -PassThru -NoNewWindow `
  -ArgumentList @('serve', '--data-dir', "`"$board`"", '--bind', "127.0.0.1:$port")
try {
  $base = "http://127.0.0.1:$port"
  for ($i = 0; $i -lt 30; $i++) {
    try { Invoke-WebRequest "$base/healthz" -UseBasicParsing -TimeoutSec 2 | Out-Null; break } catch { Start-Sleep 1 }
  }
  $body = (Invoke-WebRequest "$base/" -UseBasicParsing).Content
  if ($body -notmatch '/assets/') {
    Write-Host '::error::the installed binary served no monitor — the embed did not ship'; exit 1
  }
  if ($body -notmatch 'src="/assets/([^"]+)"') {
    Write-Host '::error::index.html names no bundle'; exit 1
  }
  try { Invoke-WebRequest "$base/assets/$($Matches[1])" -UseBasicParsing | Out-Null }
  catch { Write-Host '::error::the bundle the index names is not served'; exit 1 }
  Write-Host "installed, names $tag at $sha, monitor served, bundle intact."
}
finally {
  Stop-Process -Id $server.Id -Force -ErrorAction SilentlyContinue
}
