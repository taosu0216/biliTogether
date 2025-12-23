#!/bin/bash

# VO Android APK 一键构建+签名脚本
# 自动完成：清理 → 构建 → 签名 → 输出可安装的 APK

set -e

echo "=========================================="
echo "🚀 VO APK 一键构建工具"
echo "=========================================="

# 设置 JAVA_HOME
export JAVA_HOME="/Applications/Android Studio.app/Contents/jbr/Contents/Home"
export PATH="$JAVA_HOME/bin:$PATH"

# 1. 清理旧的构建产物
echo ""
echo "🧹 清理旧的构建产物..."
rm -rf src-tauri/gen/android/app/src/main/jniLibs/*
rm -rf src-tauri/gen/android/app/build
rm -f vo_release_unsigned.apk
rm -f vo_signed.apk

echo "✅ 清理完成"

# 2. 构建 Release APK (仅 arm64 架构)
echo ""
echo "📦 开始构建 Release APK (arm64-v8a)..."
echo "这可能需要几分钟，请耐心等待..."
echo ""

pnpm tauri android build --target aarch64

if [ $? -ne 0 ]; then
    echo "❌ 构建失败"
    exit 1
fi

# 3. 复制 unsigned APK
APK_SOURCE="src-tauri/gen/android/app/build/outputs/apk/universal/release/app-universal-release-unsigned.apk"
APK_UNSIGNED="vo_release_unsigned.apk"
APK_SIGNED="vo_signed.apk"

if [ ! -f "$APK_SOURCE" ]; then
    echo "❌ 错误: 找不到构建产物"
    echo "预期位置: $APK_SOURCE"
    exit 1
fi

cp "$APK_SOURCE" "$APK_UNSIGNED"
echo "✅ APK 构建完成"
echo "未签名版本: $APK_UNSIGNED ($(du -h "$APK_UNSIGNED" | cut -f1))"

# 4. 签名 APK
echo ""
echo "🔐 正在签名 APK..."

KEYSTORE="$HOME/.android/debug.keystore"
KEY_ALIAS="androiddebugkey"
KEY_PASSWORD="android"
STORE_PASSWORD="android"

# 检查 debug keystore 是否存在，不存在则创建
if [ ! -f "$KEYSTORE" ]; then
    echo "📝 创建 debug.keystore..."
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

# 查找 apksigner
APKSIGNER=""
if command -v apksigner &> /dev/null; then
    APKSIGNER="apksigner"
elif [ -d "$HOME/Library/Android/sdk/build-tools" ]; then
    BUILD_TOOLS_VERSION=$(ls -1 "$HOME/Library/Android/sdk/build-tools" | sort -V | tail -n 1)
    APKSIGNER="$HOME/Library/Android/sdk/build-tools/$BUILD_TOOLS_VERSION/apksigner"
fi

if [ -z "$APKSIGNER" ] || [ ! -f "$APKSIGNER" ]; then
    echo "❌ 错误: 找不到 apksigner 工具"
    exit 1
fi

# 签名
"$APKSIGNER" sign \
    --ks "$KEYSTORE" \
    --ks-key-alias "$KEY_ALIAS" \
    --ks-pass pass:"$STORE_PASSWORD" \
    --key-pass pass:"$KEY_PASSWORD" \
    --out "$APK_SIGNED" \
    "$APK_UNSIGNED"

# 验证签名
"$APKSIGNER" verify "$APK_SIGNED" > /dev/null 2>&1

if [ $? -eq 0 ]; then
    echo "✅ 签名成功"
else
    echo "❌ 签名验证失败"
    exit 1
fi

mv /Users/taosu/workspace/tauri/video/vo/vo_signed.apk /Users/taosu/workspace/tauri/video/vo/third/vo_signed.apk

# 5. 输出结果
echo ""
echo "=========================================="
echo "🎉 构建完成!"
echo "=========================================="
echo "📱 可安装的 APK: $APK_SIGNED"
echo "📦 文件大小: $(du -h "$APK_SIGNED" | cut -f1)"
echo ""
echo "📲 安装方法:"
echo "  1. 传到手机: 微信/AirDrop/网盘"
echo "  2. 手机上点击安装"
echo "  3. 允许「未知来源」安装"
echo ""
echo "或使用 adb 安装:"
echo "  adb install -r $APK_SIGNED"
echo "=========================================="

