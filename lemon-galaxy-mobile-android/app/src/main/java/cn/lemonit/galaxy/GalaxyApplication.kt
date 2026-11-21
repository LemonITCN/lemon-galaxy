package cn.lemonit.galaxy

import android.app.Application
import cn.lemonit.galaxy.app.AppContainer

class GalaxyApplication : Application() {
    val container: AppContainer by lazy { AppContainer.create(this) }
}
