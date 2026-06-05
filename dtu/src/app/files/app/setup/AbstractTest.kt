package c.arve

import android.content.Context
import android.os.Bundle

abstract class AbstractTest(
    protected val context: Context
) : IDeviceTest.Stub() {

    protected var logger: AbstractLogger = AndroidLogger()

    abstract fun doTest(extras: Bundle?): Boolean
    open fun cleanup() { }

    override fun runTest(data: Bundle?, logger: ILogger?): Boolean {
        if (logger != null) {
            this.logger = WrappedLogger(logger)
        }
        val success = try {
            doTest(data)
        } catch (e : Exception) {
            this.logger.error("unhandled exception", e)
            false
        }

        try {
            cleanup()
        } catch (e : Exception) {
            this.logger.error("cleanup failed with exception", e)
        }

        return success
    }

}
