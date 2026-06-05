package c.arve

import android.app.Application
import android.content.Intent
import androidx.appcompat.app.AppCompatDelegate

class App : Application() {
    override fun onCreate() {
        AppCompatDelegate.setDefaultNightMode(AppCompatDelegate.MODE_NIGHT_YES)
        super.onCreate()
        try {
            startService(
                Intent(this, Server::class.java)
            )
        } catch (e: Exception) {
            alogw("failed to start application server")
        }
    }
}
