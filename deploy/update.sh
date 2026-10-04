#!/usr/bin/env bash
# 中央ノードで、普段Ansibleを実行するユーザーから呼び出します。
set -Eeuo pipefail

usage() {
    cat <<'HELP'
使い方
  ./deploy/update.sh VERSION [オプション] [-- Ansibleのオプション]

中央ノードを先に更新し、起動を確認してから下流ノードを更新します。
更新時に監視状態と未解決の障害をすべて初期化し、新しい観測から判定し直します。
VERSIONには更新先の公開済みバージョンをvX.Y.Zの形式で指定します。
先頭のvは省略できます。state resetに対応したバイナリが必要です。

オプション
  --parent-only       中央ノードだけを更新する
  --agents-only       中央ノードのバージョンと起動を確認し、下流だけを更新する
  --inventory PATH    既存のインベントリを使う。既定はdeploy/ansible/inventory.ini
  --repo OWNER/REPO   バイナリの取得元。既定はmizuno-group/cluster-sentinel
  --help              この説明を表示する

実行例
  ./deploy/update.sh vX.Y.Z -- -K
  ./deploy/update.sh vX.Y.Z -- --ask-vault-pass -K
  ./deploy/update.sh vX.Y.Z --parent-only
  ./deploy/update.sh vX.Y.Z --agents-only -- -k -K --limit node01

中央ノードでは/usr/local/bin/sentinelと/etc/sentinel/config.tomlを使います。
実行中のsentinel-agentが中央ノードにあれば、そのサービスも再起動します。
Ansibleは実行したユーザーのSSH設定を使います。スクリプト全体をsudoで
実行せず、sudoが必要な処理では求められたパスワードを入力してください。
HELP
}

die() {
    printf '更新を中止しました。%s\n' "$*" >&2
    exit 1
}

need_value() {
    [[ $# -ge 2 && -n $2 && $2 != -* ]] || die "$1の値を指定してください。"
}

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
ansible_dir="$script_dir/ansible"
inventory="$ansible_dir/inventory.ini"
repo=mizuno-group/cluster-sentinel
binary=/usr/local/bin/sentinel
config=/etc/sentinel/config.toml
mode=all
version=
ansible_args=()

while (($#)); do
    case "$1" in
        --help|-h) usage; exit 0 ;;
        --parent-only|--agents-only)
            [[ $mode == all ]] || die '--parent-onlyと--agents-onlyは同時に指定できません。'
            mode=${1#--}
            shift ;;
        --inventory)
            need_value "$@"
            inventory=$2
            shift 2 ;;
        --repo)
            need_value "$@"
            repo=$2
            shift 2 ;;
        --)
            shift
            ansible_args=("$@")
            break ;;
        -*) die "不明なオプションです。$1。Ansibleのオプションは--の後に指定してください。" ;;
        *)
            [[ -z $version ]] || die 'バージョンは1つだけ指定してください。'
            version=$1
            shift ;;
    esac
done

[[ $version =~ ^v?[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z][0-9A-Za-z.-]*)?$ ]] ||
    die '更新先のバージョンをvX.Y.Zの形式で指定してください。'
version="v${version#v}"
[[ $repo =~ ^[0-9A-Za-z_.-]+/[0-9A-Za-z_.-]+$ ]] || die '--repoはOWNER/REPOの形式で指定してください。'
[[ -z ${SUDO_USER:-} || ${SUDO_USER} == root ]] || die 'sudoを付けず、普段Ansibleを実行するユーザーから起動してください。'

for command_name in sudo systemctl; do
    command -v "$command_name" >/dev/null || die "$command_nameが見つかりません。"
done

if [[ $mode != parent-only ]]; then
    command -v ansible-playbook >/dev/null || die 'ansible-playbookが見つかりません。中央ノードにAnsibleをインストールしてください。'
    [[ -r $inventory && -f $inventory ]] || die "既存のインベントリが見つかりません。--inventoryで指定してください。$inventory"
    [[ -r $ansible_dir/site.yml ]] || die "Ansibleのplaybookが見つかりません。$ansible_dir/site.yml"
    # この後でAnsibleのディレクトリへ移るため、相対パスを絶対パスにします。
    inventory="$(cd -- "$(dirname -- "$inventory")" && pwd)/$(basename -- "$inventory")"
    for argument in "${ansible_args[@]}"; do
        case "$argument" in
            --check|-C|--syntax-check|--list-hosts|--list-tasks|--list-tags|--version|--help|-h)
                die "更新を実行しないAnsibleのオプションは指定できません。$argument。確認だけの場合はansible-playbookを直接実行してください。" ;;
        esac
    done
elif ((${#ansible_args[@]})); then
    die '--parent-onlyではAnsibleのオプションを指定できません。'
fi

download_dir=
staged_binary=
staged_backup=
replaced=false
controller_stopped=false
phase='更新の準備'

cleanup() {
    local result=$?
    trap - EXIT
    [[ -z $download_dir ]] || rm -rf -- "$download_dir"
    [[ -z $staged_binary ]] || sudo rm -f -- "$staged_binary"
    [[ -z $staged_backup ]] || sudo rm -f -- "$staged_backup"
    if ((result != 0)) && [[ $replaced == true ]]; then
        printf '中央ノードのバイナリは%sへ置き換え済みです。自動では元に戻しません。\n前のバイナリは%s.previousに保存しています。\n' "$version" "$binary" >&2
    fi
    if ((result != 0)) && [[ $controller_stopped == true ]]; then
        printf '中央ノードのcontrollerは停止したままです。原因を確認してからサービスを再開してください。\n' >&2
    fi
    exit "$result"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
trap 'printf "%sに失敗しました。\n" "$phase" >&2' ERR

sudo -v
sudo test -f "$binary" || die "中央ノードのバイナリが見つかりません。$binary"
sudo test -f "$config" || die "中央ノードの設定が見つかりません。$config"
[[ $(sudo systemctl show sentinel-controller.service --property=LoadState --value) == loaded ]] ||
    die 'sentinel-controller.serviceがありません。中央ノードで実行してください。'

read_version() {
    local output name installed
    output=$(sudo -u sentinel "$1" version) || die "$1のバージョンを確認できません。"
    read -r name installed _ <<< "$output"
    [[ $name == sentinel && -n $installed ]] || die "$1のバージョンを確認できません。"
    printf '%s\n' "$installed"
}

if [[ $mode != agents-only ]]; then
    for command_name in curl sha256sum mktemp chmod install cmp cp mv uname sleep getent id; do
        command -v "$command_name" >/dev/null || die "$command_nameが見つかりません。"
    done
    [[ $(uname -s) == Linux ]] || die 'このスクリプトはLinux用です。'
    architecture=$(uname -m)
    case "$architecture" in
        x86_64|aarch64) ;;
        *) die "対応していないCPUです。$architecture" ;;
    esac
    artefact="sentinel-${architecture}-unknown-linux-musl"
    url="https://github.com/$repo/releases/download/$version"
    download_dir=$(mktemp -d /tmp/sentinel-update.XXXXXXXX)
    # sentinelユーザーが検証用のバイナリを実行できるようにします。
    chmod 755 "$download_dir"
    printf '中央ノードを%sへ更新します。取得元は%sです。\n' "$version" "$repo"
    phase='リリースファイルのダウンロード'
    curl --fail --show-error --silent --location --retry 3 --connect-timeout 15 \
        --output "$download_dir/$artefact" "$url/$artefact"
    curl --fail --show-error --silent --location --retry 3 --connect-timeout 15 \
        --output "$download_dir/$artefact.sha256" "$url/$artefact.sha256"
    # チェックサムに書かれたパスを使わず、取得したファイルだけを検証します。
    read -r expected_hash _ < "$download_dir/$artefact.sha256"
    [[ $expected_hash =~ ^[0-9a-fA-F]{64}$ ]] || die 'チェックサムの形式が正しくありません。'
    read -r actual_hash _ < <(sha256sum "$download_dir/$artefact")
    [[ ${expected_hash,,} == "$actual_hash" ]] || die 'チェックサムが一致しません。現在のバイナリは変更していません。'
    chmod 755 "$download_dir/$artefact"
    [[ $(read_version "$download_dir/$artefact") == "${version#v}" ]] || die '取得したバイナリのバージョンが指定と異なります。'
    phase='新しいバイナリでの設定の検証'
    sudo -u sentinel "$download_dir/$artefact" --config "$config" config check
    sudo -u sentinel "$download_dir/$artefact" state reset --help >/dev/null ||
        die '更新先のバイナリは監視状態の初期化に対応していません。対応したリリースを指定してください。'

    phase='システムログの閲覧権限の設定'
    if getent group systemd-journal >/dev/null; then
        service_groups=$(id -nG sentinel)
        case " $service_groups " in
            *" systemd-journal "*) ;;
            *)
                sudo usermod -aG systemd-journal sentinel
                printf 'sentinelユーザーにシステムログの閲覧権限を追加しました。\n'
                ;;
        esac
    else
        printf 'systemd-journalグループがありません。ログの閲覧権限を確認してください。\n' >&2
    fi

    restart_agent=false
    if sudo systemctl is-active --quiet sentinel-agent.service; then
        restart_agent=true
    fi
    if ! sudo cmp -s -- "$download_dir/$artefact" "$binary"; then
        phase='中央ノードのバイナリの保存と置き換え'
        # 同じディレクトリ内でrenameし、実行中のファイルへの書き込みを避けます。
        staged_backup=$(sudo mktemp "${binary}.backup.XXXXXXXX")
        sudo cp -p -- "$binary" "$staged_backup"
        sudo mv -fT -- "$staged_backup" "${binary}.previous"
        staged_backup=
        staged_binary=$(sudo mktemp "${binary}.update.XXXXXXXX")
        sudo install -o root -g root -m 0755 -- "$download_dir/$artefact" "$staged_binary"
        sudo mv -fT -- "$staged_binary" "$binary"
        staged_binary=
        replaced=true
        printf '前のバイナリを%s.previousに保存しました。\n' "$binary"
    else
        printf '中央ノードには同じバイナリが入っています。サービスを再起動して確認します。\n'
    fi
fi

[[ $(read_version "$binary") == "${version#v}" ]] || die "中央ノードが$versionではありません。中央ノードを先に更新してください。"
if [[ $mode == agents-only ]]; then
    sudo systemctl is-active --quiet sentinel-controller.service || die '中央ノードが起動していません。下流の更新は実行しません。'
    sudo -u sentinel "$binary" state reset --help >/dev/null ||
        die '中央ノードのバイナリは監視状態の初期化に対応していません。中央ノードを先に更新してください。'
fi
phase='中央ノードの監視状態の初期化'
sudo systemctl stop sentinel-controller.service
controller_stopped=true
sudo -u sentinel "$binary" --config "$config" state reset
phase='中央ノードのサービスの再起動'
sudo systemctl restart sentinel-controller.service
controller_stopped=false
sudo systemctl is-active --quiet sentinel-controller.service
sleep 3
sudo systemctl is-active --quiet sentinel-controller.service
if [[ ${restart_agent:-false} == true ]]; then
    sudo systemctl restart sentinel-agent.service
    sudo systemctl is-active --quiet sentinel-agent.service
fi

show_status() {
    local result=0
    sudo systemctl is-active --quiet sentinel-controller.service || die '中央ノードのサービスが停止しています。'
    sudo -u sentinel "$binary" --config "$config" status || result=$?
    # 終了コード2は監視対象の異常です。更新処理の失敗とは区別します。
    case "$result" in
        0) ;;
        2) printf '監視対象の異常が報告されています。表示された内容を確認してください。\n' ;;
        *) die "中央ノードの状態を読み取れません。終了コードは$resultです。" ;;
    esac
}

show_status
if [[ $mode != parent-only ]]; then
    printf '下流ノードを%sへ更新します。インベントリは%sです。\n' "$version" "$inventory"
    phase='Ansibleによる下流ノードの更新'
    # SSH設定とVaultを普段どおり使い、全ノードの取得元とバージョンをそろえます。
    (
        cd -- "$ansible_dir"
        ansible-playbook -i "$inventory" site.yml "${ansible_args[@]}" \
            -e "sentinel_version=$version" -e "sentinel_repo=$repo"
    )
    show_status
fi
printf '%sへの更新が完了しました。\n' "$version"
