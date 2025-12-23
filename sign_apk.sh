#!/bin/bash

# VO Android APK 签名脚本
# 用于给未签名的 release APK 添加调试签名，方便安装测试

set -e

# 设置 JAVA_HOME（使用 Android Studio 自带的 JDK）
export JAVA_HOME="/Applications/Android Studio.app/Contents/jbr/Contents/Home"
export PATH="$JAVA_HOME/bin:$PATH"

APK_INPUT="vo_release_unsigned.apk"
APK_OUTPUT="vo_signed.apk"
KEYSTORE="$HOME/.android/debug.keystore"
KEY_ALIAS="androiddebugkey"
KEY_PASSWORD="android"
STORE_PASSWORD="android"

echo "=========================================="
echo "VO APK 签名工具"
echo "=========================================="

# 检查输入文件
if [ ! -f "$APK_INPUT" ]; then
    echo "❌ 错误: 找不到 $APK_INPUT"
    echo "请先运行: pnpm tauri android build --target aarch64"
    exit 1
fi

# 检查 debug keystore 是否存在，不存在则创建
if [ ! -f "$KEYSTORE" ]; then
    echo "📝 未找到 debug.keystore，正在创建..."
    mkdir -p "$HOME/.android"
    keytool -genkey -v -keystore "$KEYSTORE" \
        -alias "$KEY_ALIAS" \
        -keyalg RSA \
        -keysize 2048 \
        -validity 10000 \
        -storepass "$STORE_PASSWORD" \
        -keypass "$KEY_PASSWORD" \
        -dname "CN=Android Debug,O=Android,C=US"
    echo "✅ debug.keystore 创建完成"
fi

# 检查 apksigner 工具
APKSIGNER=""
if command -v apksigner &> /dev/null; then
    APKSIGNER="apksigner"
elif [ -f "/Applications/Android Studio.app/Contents/jbr/Contents/Home/bin/apksigner" ]; then
    APKSIGNER="/Applications/Android Studio.app/Contents/jbr/Contents/Home/bin/apksigner"
elif [ -d "$HOME/Library/Android/sdk/build-tools" ]; then
    # 查找最新版本的 build-tools
    BUILD_TOOLS_VERSION=$(ls -1 "$HOME/Library/Android/sdk/build-tools" | sort -V | tail -n 1)
    APKSIGNER="$HOME/Library/Android/sdk/build-tools/$BUILD_TOOLS_VERSION/apksigner"
fi

if [ -z "$APKSIGNER" ] || [ ! -f "$APKSIGNER" ]; then
    echo "❌ 错误: 找不到 apksigner 工具"
    echo "请确保已安装 Android SDK Build Tools"
    exit 1
fi

echo "📦 正在签名 APK..."
echo "输入: $APK_INPUT ($(du -h "$APK_INPUT" | cut -f1))"

# 删除旧的输出文件
rm -f "$APK_OUTPUT"

# 签名 APK
"$APKSIGNER" sign \
    --ks "$KEYSTORE" \
    --ks-key-alias "$KEY_ALIAS" \
    --ks-pass pass:"$STORE_PASSWORD" \
    --key-pass pass:"$KEY_PASSWORD" \
    --out "$APK_OUTPUT" \
    "$APK_INPUT"

# 验证签名
echo "🔍 验证签名..."
"$APKSIGNER" verify "$APK_OUTPUT"

if [ $? -eq 0 ]; then
    echo ""
    echo "=========================================="
    echo "✅ 签名成功!"
    echo "=========================================="
    echo "输出文件: $APK_OUTPUT"
    echo "文件大小: $(du -h "$APK_OUTPUT" | cut -f1)"
    echo ""
    echo "📱 安装方法:"
    echo "1. 将 $APK_OUTPUT 传到手机"
    echo "2. 在手机上打开文件管理器，点击安装"
    echo "3. 如提示「未知来源」，请在设置中允许安装"
    echo ""
    echo "或使用 adb 安装:"
    echo "  adb install -r $APK_OUTPUT"
    echo "=========================================="
else
    echo "❌ 签名验证失败"
    exit 1
fi

