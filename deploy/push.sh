#!/bin/bash
# 一键提交并推送到 Gitea（每次修改后运行即可）
# Usage: ./deploy/push.sh ["commit message"]
#
# 远端说明 / Remote:
#   - origin 已在本地 .git/config 配置（含凭据，不入库）；本脚本不硬编码任何地址
#   - 如需改用其他远端（如 SSH <gitea-host>:<port>/OM/zbxpatrol.git），设 ZBP_PUSH_REMOTE 覆盖
set -e
cd "$(dirname "$0")/.."

MSG="${1:-chore: update}"
git add -A
if git diff --cached --quiet; then
  echo "没有变更需要提交 / nothing to commit"
else
  git commit -m "$MSG"
fi

if [ -n "$ZBP_PUSH_REMOTE" ]; then
  # SSH 方式（网络可达 2222 时）
  GIT_SSH_COMMAND="sshpass -p ${ZBP_GIT_PASS:?need ZBP_GIT_PASS} ssh -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null" \
    git push -u "$ZBP_PUSH_REMOTE" main
else
  git push -u origin main
fi
