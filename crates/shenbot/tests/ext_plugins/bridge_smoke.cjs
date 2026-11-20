// 可选三方冒烟：真实 shenbot、真实 SDK 插件、仅监听 127.0.0.1 的虚构 Bridge。
const assert = require('node:assert/strict')
const { spawn } = require('node:child_process')
const { EventEmitter, once } = require('node:events')
const { copyFileSync, mkdirSync, mkdtempSync, realpathSync, rmSync, writeFileSync } = require('node:fs')
const { createServer } = require('node:http')
const { tmpdir } = require('node:os')
const { join } = require('node:path')

const [socketIo, botExecutable, fixtureExecutable] = process.argv.slice(2)
if (!socketIo || !botExecutable || !fixtureExecutable) {
    console.error('用法：node bridge_smoke.cjs <socket.io 入口> <shenbot 可执行文件> <ext_plugins 测试可执行文件>')
    process.exit(2)
}
const { Server } = require(realpathSync(socketIo))
const bot = realpathSync(botExecutable)
const fixture = realpathSync(fixtureExecutable)
const roomId = -42
const sender = 20001
const botId = 10001
const pictureUrl = 'https://fixture.invalid/image'
const png = 'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+/l2sAAAAASUVORK5CYII='

function message(id, content) {
    return { _id: id, senderId: sender, username: '虚构管理员', content, time: Date.now(), files: [] }
}

async function main() {
    const dir = mkdtempSync(join(tmpdir(), 'shenbot-ext-loopback-'))
    const uploads = []
    const server = createServer((request, response) => {
        if (request.method !== 'POST' || request.url !== '/api/fixture-token/sendMessage') {
            response.writeHead(404).end()
            return
        }
        let body = ''
        request.setEncoding('utf8').on('data', (chunk) => { body += chunk })
        request.on('end', () => {
            try {
                uploads.push(JSON.parse(body))
                response.writeHead(202).end()
            } catch {
                response.writeHead(400).end()
            }
        })
    })
    const io = new Server(server)
    const mediaRequests = []
    const sends = []
    const received = new EventEmitter()
    let child
    let exited
    let log = ''
    try {
        io.on('connection', (socket) => {
            socket.on('sendMessage', (wire) => {
                sends.push(wire)
                received.emit('send')
            })
            socket.on('requestToken', (...args) => args.at(-1)('fixture-token'))
            socket.on('getNTPicURLbyFileid', (id, appId, ack) => {
                mediaRequests.push([id, appId])
                ack(pictureUrl)
            })
            socket.emit('onlineData', {
                bkn: 0, nick: '虚构宿主', uin: botId, online: true,
                sysInfo: 'icalingua-bridge-oicq 2.26.10\nRunning on loopback\nResident Set Size n/a\nHeap used n/a\nLoad n/a\nServer Node test\n1 clients connected',
            })
            socket.emit('setAllRooms', [{
                roomId, roomName: '环回测试', index: 0, unreadCount: 0, priority: 3,
                utime: Date.now(), lastMessage: { content: '' }, at: false,
            }])
        })
        const connection = once(io, 'connection', { signal: AbortSignal.timeout(15_000) })
        server.listen(0, '127.0.0.1')
        await once(server, 'listening')
        writeFileSync(join(dir, 'fixture-image.png'), Buffer.from(png, 'base64'))
        const config = join(dir, 'test.toml')
        writeFileSync(config, [
            'enable_ica = true', 'enable_py = false', 'enable_ext = true',
            '[py]', 'plugin_path = "test-plugins"',
            '[ica]', 'private_key = ""', 'host = "http://127.0.0.1:' + server.address().port + '"',
            'self_id = ' + botId, 'admin_list = [' + sender + ']', 'allow_rooms = [' + roomId + ']',
            '[ext]', 'shadow_dir = ".ext_runtime"', 'default_drain_timeout_ms = 3000',
        ].join('\n'))
        // Python 未启用时仍沿用它的插件目录，且相对 binary 以 unit 目录为基准。
        const plugins = join(dir, 'test-plugins')
        mkdirSync(plugins)
        copyFileSync(fixture, join(plugins, 'fixture.exe'))
        writeFileSync(join(plugins, 'ignored.plugin.toml'), '不是有效的 TOML')
        writeFileSync(join(plugins, 'ext_fixture.plugins.toml'), [
            'enabled = true',
            'binary = "fixture.exe"',
            'args = ["--ext-fixture", "plain", ' + JSON.stringify(dir) + ']',
        ].join('\n'))
        child = spawn(bot, ['-c', config], {
            cwd: dir, stdio: ['ignore', 'pipe', 'pipe'],
            env: { ...process.env, NO_PROXY: '127.0.0.1,localhost' },
        })
        child.on('error', (error) => { log += error.message })
        exited = new Promise((resolve) => child.once('close', resolve))
        for (const stream of [child.stdout, child.stderr]) {
            stream.setEncoding('utf8').on('data', (text) => { log = (log + text).slice(-8000) })
        }
        const [socket] = await Promise.race([connection, exited.then(() => { throw Error('宿主过早退出') })])
        const add = (id, content) => socket.emit('addMessage', { roomId, message: message(id, content) })
        const nextSend = async (match) => {
            const signal = AbortSignal.timeout(7000)
            while (!sends.some(match)) await once(received, 'send', { signal })
            return sends.find(match)
        }

        socket.emit('setMessages', { roomId, messages: [message('history', '/fixture history')] })
        socket.emit('addMessage', { roomId: -43, message: message('other-room', '/bot-rs') })
        add('non-command', '与插件订阅无关')
        add('fixture-message', '/fixture ping')
        const pong = await nextSend((entry) => entry.content.endsWith(':pong'))
        assert.equal(pong.roomId, roomId)
        assert.equal(pong.replyMessage._id, 'fixture-message')
        assert.equal(pong.replyMessage.senderId, sender)

        add('picture', '/fixture picture')
        await nextSend((entry) => entry.content.endsWith(pictureUrl))
        assert.deepEqual(mediaRequests, [['fake-nt-image', '']])
        add('image', '/fixture image')
        await nextSend((entry) => entry.content.endsWith(':image-acked'))
        assert.equal(uploads.length, 1)
        assert.equal(uploads[0].roomId, roomId)
        assert.equal(uploads[0].replyMessage._id, 'fixture-message')
        assert.equal(uploads[0].media[0].b64, 'data:image/png;base64,' + png)

        add('list', '/bot-ls')
        await nextSend((entry) => entry.content.includes('[ext] ext_fixture'))
        add('version', '/bot-rs')
        const version = await nextSend((entry) => entry.replyMessage?._id === 'version')
        const clientId = version.content.match(/\[(\d{6})\]/)?.[1]
        assert.ok(clientId, '版本信息包含六位宿主管理 ID')
        add('disable', '/bot-disable-' + clientId + ' ext_fixture')
        await nextSend((entry) => entry.content.endsWith(':shutdown'))
        await nextSend((entry) => entry.content === '禁用插件完成')
        assert.ok(!sends.some((entry) => entry.content.includes('unexpected-routing')), '历史和无关实时消息未派发')
        assert.ok(sends.every((entry) => entry.roomId === roomId), '宿主没有响应未授权房间')
        console.log('环回 Bridge：订阅过滤、同房间引用、NT 图片查询、HTTP 图片 ACK、管理命令与插件排空均通过')
    } catch (error) {
        console.error(log)
        throw error
    } finally {
        if (child) {
            child.kill()
            await exited
        }
        await new Promise((resolve) => io.close(resolve))
        rmSync(dir, { recursive: true, force: true, maxRetries: 5 })
    }
}

main().catch((error) => {
    console.error(error)
    process.exitCode = 1
})
