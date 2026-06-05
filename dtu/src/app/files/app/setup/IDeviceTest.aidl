package c.arve;

import c.arve.ILogger;
import android.os.Bundle;

interface IDeviceTest {
    // Runs a test with optional bundle data and returns whether or not
    // the test was successful.
    boolean runTest(in Bundle data, ILogger logger);
}
