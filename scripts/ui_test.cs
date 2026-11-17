using System;
using System.Runtime.InteropServices;
using System.Threading;
using System.Windows.Forms;

public class UiTest {
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hWnd, out RECT r);
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern void mouse_event(uint dwFlags, int dx, int dy, uint dwData, UIntPtr dwExtraInfo);
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }

    static void Click(int x, int y) {
        SetCursorPos(x, y);
        Thread.Sleep(300);
        mouse_event(0x0002, 0, 0, 0, UIntPtr.Zero);
        Thread.Sleep(60);
        mouse_event(0x0004, 0, 0, 0, UIntPtr.Zero);
        Thread.Sleep(500);
    }

    public static void Main(string[] args) {
        IntPtr h = (IntPtr)long.Parse(args[0]);
        SetForegroundWindow(h);
        Thread.Sleep(800);
        RECT r; GetWindowRect(h, out r);
        int ox = r.Left, oy = r.Top;

        // Click the tag input field (found at window-relative ~(610-ox), 212-oy))
        Click(610 + 60, 212 + 15);
        Thread.Sleep(400);
        SendKeys.SendWait("work-test");
        Thread.Sleep(400);
        SendKeys.SendWait("{ENTER}");
        Thread.Sleep(800);

        Console.WriteLine("ui-test done");
    }
}
