#!/usr/bin/env bash
# Reads the existing configuration; never prints proxy credentials or changes it.
set -euo pipefail
SERVICE_DIR="$(cd "$(dirname "$0")" && pwd -P)"
cd "$SERVICE_DIR"
[[ ! -f settings.env ]] || source settings.env
command -v curl >/dev/null || { echo '缺少 curl，请先安装后重试。'; exit 1; }
if [[ -z "${DEEPLX_PROXY:-}" ]]; then
  echo '未读取到 DEEPLX_PROXY；请确认修改的是当前实例目录里的 settings.env。'
  exit 1
fi
if [[ "$DEEPLX_PROXY" == *$'\n'* || "$DEEPLX_PROXY" == *$'\r'* ]]; then
  echo '代理配置包含换行，无法使用。'
  exit 1
fi

umask 077
CHECK_DIR="$(mktemp -d "${TMPDIR:-/tmp}/crantpeek-deeplx-check.XXXXXXXX")"
trap 'rm -rf -- "$CHECK_DIR"' EXIT
quote_config() {
  local value="$1"
  value="${value//\\/\\\\}"
  value="${value//\"/\\\"}"
  printf '"%s"' "$value"
}
{
  printf 'proxy = '; quote_config "$DEEPLX_PROXY"; printf '\nnoproxy = ""\n'
} > "$CHECK_DIR/proxy.conf"
printf 'proxy = ""\nnoproxy = "*"\n' > "$CHECK_DIR/direct.conf"
cp "$CHECK_DIR/direct.conf" "$CHECK_DIR/local.conf"
if [[ -n "${DEEPLX_TOKEN:-}" ]]; then
  printf 'header = ' >> "$CHECK_DIR/local.conf"
  quote_config "Authorization: Bearer $DEEPLX_TOKEN" >> "$CHECK_DIR/local.conf"
  printf '\n' >> "$CHECK_DIR/local.conf"
fi

echo '=== DeepLX 自检（不显示代理地址、账号、密码） ==='
echo '配置：已读取代理。'
found_process=false
if [[ -d /proc/self ]]; then
  for executable in /proc/[0-9]*/exe; do
    target="$(readlink "$executable" 2>/dev/null || true)"
    if [[ "$target" != "$SERVICE_DIR/deeplx_linux_amd64" && "$target" != "$SERVICE_DIR/deeplx_linux_amd64 (deleted)" ]]; then continue; fi
    found_process=true
    if command -v sha256sum >/dev/null && [[ -r SHA256SUMS.txt ]]; then
      expected="$(awk 'NR == 1 {print $1}' SHA256SUMS.txt)"
      actual="$(sha256sum "$executable" 2>/dev/null | awk '{print $1}' || true)"
      if [[ -n "$actual" && "$actual" == "$expected" ]]; then
        echo '运行中程序：与本目录的上游交付校验记录一致。'
      elif [[ -n "$actual" ]]; then
        echo '运行中程序：与上游交付校验记录不同，可能使用了旧版或其他构建。'
      fi
    fi
    process_proxy=''
    if [[ -r "${executable%/exe}/environ" ]]; then
      while IFS= read -r -d '' entry; do
        if [[ "$entry" == PROXY=* ]]; then process_proxy="${entry#PROXY=}"; break; fi
      done < "${executable%/exe}/environ"
      if [[ "$process_proxy" == "$DEEPLX_PROXY" ]]; then
        echo '运行中进程：PROXY 与配置一致。'
      elif [[ -z "$process_proxy" ]]; then
        echo '运行中进程：未读取到 PROXY，请检查启动脚本并重启。'
      else
        echo '运行中进程：PROXY 与配置不一致，请确认已重启正确实例。'
      fi
    else
      echo '运行中进程：无权限读取环境，无法核对代理。'
    fi
    if [[ "$target" == *' (deleted)' ]]; then echo '运行中进程：仍使用已替换的旧程序，需要重启。'; fi
  done
fi
if [[ "$found_process" == false ]]; then echo '运行中进程：未找到此目录的程序；请确认服务已启动。'; fi

fetch() {
  local config="$1" output="$2" url="$3"
  # Proxy credentials live only in a private temporary config, not argv/logs.
  curl -q --config "$config" --silent --connect-timeout 5 --max-time 12 \
    --output "$output" --write-out '%{http_code}' "$url" 2> "$CHECK_DIR/curl-error" || true
}
direct_status="$(fetch "$CHECK_DIR/direct.conf" "$CHECK_DIR/direct-ip" 'https://api.ipify.org')"
proxy_status="$(fetch "$CHECK_DIR/proxy.conf" "$CHECK_DIR/proxy-ip" 'https://api.ipify.org')"
if [[ "$proxy_status" == 200 ]]; then
  echo '代理连通：成功。'
  if [[ "$direct_status" == 200 ]]; then
    if cmp -s "$CHECK_DIR/direct-ip" "$CHECK_DIR/proxy-ip"; then
      echo '出口比较：直连与代理出口相同；不能据此证明请求已换出口。'
    else
      echo '出口比较：直连与代理出口不同。'
    fi
  else
    echo '出口比较：直连测试失败，无法比较；不影响上面的代理连通结果。'
  fi
else
  echo "代理连通：失败（HTTP ${proxy_status}）；这是网络测试，不是翻译请求。"
fi

version_status="$(fetch "$CHECK_DIR/proxy.conf" "$CHECK_DIR/app-version" 'https://itunes.apple.com/lookup?id=1552407475&country=us')"
if [[ "$version_status" == 200 ]]; then
  version="$(sed -nE 's/.*"version"[[:space:]]*:[[:space:]]*"([0-9.]+)".*/\1/p' "$CHECK_DIR/app-version" | head -n 1)"
  if [[ -n "$version" ]]; then
    echo "App Store 版本查询：成功（${version}）；此项测试不代表 DLX 内部已选用该版本。"
  else
    echo 'App Store 版本查询：响应中未找到版本号。'
  fi
else
  echo "App Store 版本查询：失败（HTTP ${version_status}）；DLX 可能使用内置回退版本。"
fi

host="${DEEPLX_BIND:-127.0.0.1}"
[[ "$host" != 0.0.0.0 ]] || host=127.0.0.1
[[ "$host" != :: ]] || host=::1
[[ "$host" != *:* ]] || host="[$host]"
status="$(curl -q --config "$CHECK_DIR/local.conf" --silent --connect-timeout 5 --max-time 20 \
  --output "$CHECK_DIR/translation" --write-out '%{http_code}' \
  "http://$host:${DEEPLX_PORT:-1188}/translate" \
  --header 'Content-Type: application/json' \
  --data '{"text":"Hello","source_lang":"EN","target_lang":"ZH"}' \
  2> "$CHECK_DIR/curl-error" || true)"
echo "本机翻译接口：HTTP ${status}（仅测试一次）。"
case "$status" in
  200) echo '后端可用；若客户端仍失败，再检查 nginx 转发和实际访问的实例。';;
  429) echo 'DeepL 上游仍返回 429；统一的“IP 被封”提示不能排除客户端兼容性问题。';;
  401) echo '后端仍要求密钥，请核对运行中实例与 settings.env。';;
  000) echo '无法连接本机服务，请确认当前实例的监听地址、端口与运行状态。';;
  *) echo '后端返回其他错误，请结合本次请求对应的 MCSM 日志检查。';;
esac
echo '自检结束。可以发送以上输出，无需发送 settings.env 或代理账号密码。'
