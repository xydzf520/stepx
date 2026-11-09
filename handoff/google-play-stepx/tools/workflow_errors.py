"""Bounded public errors. No raw traceback, account data or host paths in the UI."""
import json
import subprocess
import urllib.error


class WorkflowError(RuntimeError):
    def __init__(self, code, message, retryable=False):
        super().__init__(message)
        self.code, self.retryable = code, retryable


def describe(error):
    if isinstance(error, WorkflowError):
        return {'code': error.code, 'message': str(error), 'retryable': error.retryable}
    detail = str(error)
    if isinstance(error, subprocess.CalledProcessError):
        detail += ' ' + str(error.stderr or '') + ' ' + str(error.stdout or '')
    rules = [
        (('Google 安装或修复正在运行',), 'operation_busy', '另一项 Google 检查或修复尚未结束，请稍后重试；当前未重复执行。', True),
        (('Connection refused', '网关未就绪', 'Staging failed'), 'gateway_unavailable', '手机系统协作服务暂不可用。请先解锁手机并打开 StepClaw，保持连接后重新检测。', True),
        (('Unsupported firmware', 'Unsupported SDK', 'Unsupported ABI', 'Native image mismatch', 'Unexpected SELinux'),
         'unsupported_firmware', '当前手机固件尚未通过 Google 安装适配验证。请先更新手机系统或桌面客户端，再重新检测；重复安装不会解决，现有数据已保留。', False),
        (('Unlock the phone', '请先解锁', 'RUNNING_UNLOCKED'), 'unlock_required', '请先解锁手机一次，保持连接后重试；无需卸载 Google 组件。', False),
        (('USB 安装', '开发者选项'), 'install_permission', '请在手机「设置 → 开发者选项」开启「USB 安装」，然后点「继续安装」。已完成的步骤会保留。', False),
        (('INSTALL_FAILED_INSUFFICIENT_STORAGE', 'No space left'), 'storage_full', '手机空间不足，请释放空间后继续安装；已安装组件和账号保留。', False),
        (('INSTALL_FAILED_USER_RESTRICTED', 'INSTALL_CANCELED_BY_USER'), 'install_permission', '手机阻止了安装，请查看手机的安装授权提示并允许，再继续安装。', False),
        (('INSTALL_FAILED_ALREADY_EXISTS', 'INSTALL_FAILED_VERSION_DOWNGRADE'), 'retained_package', '手机上仍保留该组件的安装状态或较新版本。本次未覆盖；请在手机应用设置确认主用户已启用，再重新检测。', False),
        (('INSTALL_FAILED_UPDATE_INCOMPATIBLE', 'signer', 'signature', '签名'), 'untrusted_package', '已有 Google 组件的签名未通过验证，已停止覆盖和授权。请核对组件来源；无需清除账号或重复下载。', False),
        (('Archive digest mismatch', 'APK digest mismatch'), 'download_integrity', '安装包校验失败，请重新下载安装包后继续；未使用损坏的文件。', True),
        (('Wrong device baseline', '固件与当前手机不同', '恢复记录不匹配'), 'recovery_identity', '安装恢复记录与当前设备或固件不一致。已保留记录和应用，请重新检测当前手机。', False),
        (('Unknown boot', 'Adapter changed', '启动文件已变化', 'backup differs'), 'bootstrap_conflict', '手机启动适配文件与已验证版本不同，已停止覆盖。请检查其他工具的改动，或更新客户端后重新检测。', False),
        (('Only user 0',), 'unsupported_user', '当前安装流程仅验证了手机主用户，请切回主用户并检查多用户配置后重试。', False),
        (('device offline', 'device not found', 'no devices', 'No route to host', 'unauthorized', '已离线', '连接未授权'),
         'connection', '手机连接中断或未授权。连接恢复后可继续安装，已完成的步骤和原数据保留。', True),
    ]
    for needles, code, message, retryable in rules:
        if any(word.lower() in detail.lower() for word in needles):
            return {'code': code, 'message': message, 'retryable': retryable}
    if isinstance(error, (TimeoutError, subprocess.TimeoutExpired, urllib.error.URLError, ConnectionError)):
        return {'code': 'timeout', 'message': '读取或下载超时，请检查手机连接和网络后重试；程序会重新核对已完成的步骤。', 'retryable': True}
    return {'code': 'needs_inspection', 'message': '当前步骤未通过验证，已停止后续操作。请重新检测并查看排障日志；保留已有组件、账号和恢复记录。', 'retryable': False}


def emit(error, stream):
    print('STEPX_GOOGLE_ERROR ' + json.dumps(describe(error), ensure_ascii=False), file=stream)
