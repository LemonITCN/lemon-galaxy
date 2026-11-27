package cn.lemonit.galaxy

import cn.lemonit.galaxy.app.AccountController
import cn.lemonit.galaxy.app.DeviceUpgrade
import cn.lemonit.galaxy.app.HostItem
import cn.lemonit.galaxy.app.HostTree
import cn.lemonit.galaxy.app.HostsData
import cn.lemonit.galaxy.app.RdpLinks
import cn.lemonit.galaxy.app.RemoteUpgrade
import cn.lemonit.galaxy.core.bridge.RdpTarget
import cn.lemonit.galaxy.core.bridge.FileKind
import cn.lemonit.galaxy.core.bridge.InputEvent
import cn.lemonit.galaxy.core.mock.AnnexB
import cn.lemonit.galaxy.core.mock.MockData
import cn.lemonit.galaxy.ui.files.FileBrowserController
import cn.lemonit.galaxy.ui.remote.Hid
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** 纯逻辑单元测试（不需要 Android 运行时）。 */
class LogicTest {
    @Test
    fun rdpLinkEncodesAndOmitsPassword() {
        val t = RdpTarget("Finance PC", "127.0.0.1", 53389u, "finance", "CORP", "s3cret", true, "f1")
        val uri = RdpLinks.uri(t)
        assertEquals("rdp://full%20address=s:127.0.0.1:53389&username=s:CORP%5Cfinance", uri)
        assertFalse(uri.contains("s3cret"))
        // 已经带域或是 UPN 的用户名不再拼域；IPv6 地址加方括号
        assertEquals(
            "rdp://full%20address=s:[240e:3b0::1]:3389&username=s:alice%40corp.example",
            RdpLinks.uri(RdpTarget("v6", "240e:3b0::1", 3389u, "alice@corp.example", "CORP", null, false, "")),
        )
        assertEquals("rdp://full%20address=s:host:3389", RdpLinks.uri(RdpTarget("x", "host", 3389u, "", null, null, false, "")))
    }

    @Test
    fun versionCompare() {
        assertTrue(MockData.isNewer("0.2.0", "0.1.0"))
        assertTrue(MockData.isNewer("0.10.0", "0.9.9"))
        assertFalse(MockData.isNewer("0.2.0", "0.2.0"))
        assertFalse(MockData.isNewer("0.1.9", "0.2"))
    }

    @Test
    fun serverAddressValidation() {
        assertTrue(AccountController.isValidServer("galaxy.example.com:7443#sha256-AbCdEf0123456789"))
        assertTrue(AccountController.isValidServer("10.0.0.5:7443#sha256-q3Vb1oX9mH2c7uY0kN4t"))
        assertTrue(AccountController.isValidServer("[240e:3b0::1]:7443#sha256-q3Vb1oX9mH2c7uY0kN4t"))
        assertFalse(AccountController.isValidServer("galaxy.example.com:7443"))
        assertFalse(AccountController.isValidServer("galaxy.example.com#sha256-AbCdEf0123456789"))
        assertFalse(AccountController.isValidServer("galaxy.example.com:99999#sha256-AbCdEf0123456789"))
        assertFalse(AccountController.isValidServer("galaxy.example.com:7443#md5-AbCdEf0123456789"))
        assertFalse(AccountController.isValidServer("https://galaxy.example.com:7443#sha256-AbCdEf0123456789"))
    }

    @Test
    fun hidCombo() {
        val e = Hid.combo(Hid.CTRL, Hid.ALT, Hid.DELETE)
        assertEquals(
            listOf(
                InputEvent.Key(Hid.CTRL, true), InputEvent.Key(Hid.ALT, true),
                InputEvent.Key(Hid.DELETE, true), InputEvent.Key(Hid.DELETE, false),
                InputEvent.Key(Hid.ALT, false), InputEvent.Key(Hid.CTRL, false),
            ),
            e,
        )
        assertEquals(0x04u, Hid.letter('a'))
        assertEquals(0x1Du, Hid.letter('Z'))
        assertEquals(0x45u, Hid.f(12))
    }

    @Test
    fun hostTreeFoldersAndSearch() {
        val data = HostsData(MockData.devices(), MockData.vault())
        val rows = HostTree.rows(data)
        // 第一行是 Production 文件夹，里面的 Databases 子文件夹紧随其后
        assertEquals(MockData.F_PROD, rows[0].folder?.id)
        assertEquals(MockData.F_DB, rows[1].folder?.id)
        assertEquals(1, rows[1].depth)
        // Home Lab 默认折叠：NAS 不出现
        assertFalse(rows.any { (it.item as? HostItem.Device)?.device?.id == MockData.DEV_NAS })
        // 展开后出现
        val expanded = HostTree.rows(data, expandedOverride = mapOf(MockData.F_LAB to true))
        assertTrue(expanded.any { (it.item as? HostItem.Device)?.device?.id == MockData.DEV_NAS })
        // 手机端设备不出现在可连接列表里
        assertFalse(rows.any { (it.item as? HostItem.Device)?.device?.id == MockData.DEV_SELF })
        // 搜索平铺并带文件夹路径
        val found = HostTree.rows(data, query = "replica")
        assertEquals(1, found.size)
        assertEquals("Production / Databases", found[0].path)
        // 标签筛选
        assertTrue(HostTree.rows(data, tag = "windows").all { "windows" in it.item!!.tags })
        // 收藏
        assertTrue(HostTree.rows(data, favoritesOnly = true).all { it.item!!.favorite })
        // 移动文件夹时不能移进自己或子孙
        assertEquals(setOf(MockData.F_PROD, MockData.F_DB), HostTree.descendants(data, MockData.F_PROD))
    }

    @Test
    fun remoteUpgradeOnlyForControllableDevicesWithNewVersion() {
        val all = MockData.devices().associateBy { it.id }
        fun d(id: String) = all.getValue(id)
        // worker 与 macOS 客户端（带本机被控）可升级
        assertTrue(RemoteUpgrade.available(d(MockData.DEV_WEB_01)))
        assertTrue(RemoteUpgrade.available(d(MockData.DEV_BUILD_MAC)))
        // Windows 客户端不被控：服务器报了新版本也不算
        assertFalse(RemoteUpgrade.controllable(d(MockData.DEV_OFFICE_PC)))
        assertFalse(RemoteUpgrade.available(d(MockData.DEV_OFFICE_PC)))
        // 本机（手机）不算；没有新版本的不标
        assertFalse(RemoteUpgrade.controllable(d(MockData.DEV_SELF)))
        assertFalse(RemoteUpgrade.controllable(d(MockData.DEV_SELF).copy(kind = cn.lemonit.galaxy.core.bridge.DeviceKind.WORKER)))
        assertFalse(RemoteUpgrade.available(d(MockData.DEV_DB_01)))
        assertTrue(RemoteUpgrade.controllable(d(MockData.DEV_MACBOOK)))
        assertFalse(RemoteUpgrade.available(d(MockData.DEV_MACBOOK)))
        // 离线时列表照样标，但按钮不可点
        val render = d(MockData.DEV_RENDER)
        assertTrue(RemoteUpgrade.available(render))
        assertFalse(RemoteUpgrade.canStart(render, null))
        // 在线：可点；进行中不可点；失败后（即使服务器已不报新版本）可重试
        val web = d(MockData.DEV_WEB_01)
        assertTrue(RemoteUpgrade.canStart(web, null))
        assertFalse(RemoteUpgrade.canStart(web, DeviceUpgrade.Requesting("0.1.1")))
        assertFalse(RemoteUpgrade.canStart(web, DeviceUpgrade.Installing("0.1.1")))
        assertFalse(RemoteUpgrade.canStart(web, DeviceUpgrade.Restarting("0.1.1")))
        assertTrue(RemoteUpgrade.canStart(web.copy(updateVersion = ""), DeviceUpgrade.Failed("x")))
        assertFalse(RemoteUpgrade.canStart(web.copy(updateVersion = ""), DeviceUpgrade.Done("0.1.1", false)))
        // 升级完成后服务器又报了更新的版本：详情按「有新版本」显示；已是最新的结果照常显示
        val done = DeviceUpgrade.Done("0.1.1", alreadyLatest = false)
        assertEquals(done, done.shownFor(web.copy(appVersion = "0.1.1", updateVersion = "")))
        assertEquals(null, done.shownFor(web.copy(appVersion = "0.1.1", updateVersion = "0.1.2")))
        val latest = DeviceUpgrade.Done("0.1.0", alreadyLatest = true)
        assertEquals(latest, latest.shownFor(web))
    }

    @Test
    fun permissionString() {
        assertEquals("drwxr-xr-x", FileBrowserController.permString(FileKind.DIR, 0x1EDu))
        assertEquals("-rw-------", FileBrowserController.permString(FileKind.FILE, 0x180u))
        assertEquals("lrwxrwxrwx", FileBrowserController.permString(FileKind.SYMLINK, 0x1FFu))
    }

    @Test
    fun annexBSplitsAccessUnits() {
        fun nal(type: Int, len: Int) = byteArrayOf(0, 0, 0, 1, type.toByte()) + ByteArray(len) { 0x55 }
        val aud = nal(9, 1)
        val stream = aud + nal(7, 4) + nal(8, 2) + nal(5, 10) + aud + nal(1, 8) + aud + nal(1, 8)
        val aus = AnnexB.split(stream)
        assertEquals(3, aus.size)
        assertTrue(aus[0].keyframe)
        assertFalse(aus[1].keyframe)
        assertEquals(stream.size, aus.sumOf { it.data.size })
    }
}
