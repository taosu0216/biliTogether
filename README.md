# Forked From bilitools

https://github.com/btjawa/BiliTools

当时图省事懒得 fork，这个是原仓库，我只是自己 clone 下来用 ai 二开了一下
另外借鉴了 videoTogether 的思路做的 server（代码写的没人家漂亮）
https://github.com/VideoTogether/VideoTogether
https://2gether.video/zh-cn/

---

## 快速开始

### 1. 启动 Server（桌面端）

```bash
cd third/BiliTools
pnpm install
pnpm dev
```

启动后同步服务监听 `http://localhost:18080`

### 2. 编译手机端 APK

```bash
# 项目根目录
pnpm install
./build_apk.sh
```

输出：`third/vo_signed.apk`，传到手机安装即可。

### 3. 一起看（远程同步）

需要用 Cloudflare Tunnel 暴露本地服务：

```bash
cloudflared tunnel --url http://localhost:18080
```

运行后会生成一个公网地址（如 `https://xxx.trycloudflare.com`），手机端填入该地址即可连接。

## 使用流程

1. 电脑启动 BiliTools → 登录 B 站账号
2. 运行 `cloudflared tunnel` 获取公网地址
3. 手机打开 VO App → 输入公网地址和房间信息
4. 电脑端解析视频并投送 → 手机端同步播放

---

## 免责声明

- 本项目仅供学习交流使用，请勿用于任何商业或非法用途
- 使用本项目产生的一切后果由用户自行承担，与开发者无关
- 本项目不提供任何视频资源，所有内容均来自用户自有账号
- 请遵守当地法律法规及平台服务协议
- 如有侵权请联系删除
