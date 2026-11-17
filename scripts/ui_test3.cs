using System;
using System.Runtime.InteropServices;
using System.Threading;

public class UiTest3 {
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern void mouse_event(uint f, int dx, int dy, uint d, UIntPtr e);
    [DllImport("user32.dll")] public static extern uint SendInput(uint n, INPUT[] p, int cb);

    [StructLayout(LayoutKind.Sequential)] public struct INPUT {
        public uint type;
        public KEYBDINPUT ki;
        public int pad1, pad2, pad3, pad4;
    }
    [StructLayout(LayoutKind.Sequential)] public struct KEYBDINPUT {
        public ushort wVk, wScan;
        public uint dwFlags, time;
        public IntPtr dwExtraInfo;
    }

    static void Click(int x, int y) {
        SetCursorPos(x, y);
        Thread.Sleep(350);
        mouse_event(0x0002, 0, 0, 0, UIntPtr.Zero);
        Thread.Sleep(80);
        mouse_event(0x0004, 0, 0, 0, UIntPtr.Zero);
        Thread.Sleep(600);
    }

    static void TypeChar(char c) {
        var down = new INPUT { type = 1, ki = new KEYBDINPUT { wScan = c, dwFlags = 4 } };
        var up = new INPUT { type = 1, ki = new KEYBDINPUT { wScan = c, dwFlags = 4 | 2 } };
        SendInput(2, new INPUT[] { down, up }, Marshal.SizeOf(typeof(INPUT)));
        Thread.Sleep(50);
    }

    static void Key(ushort vk) {
        var down = new INPUT { type = 1, ki = new KEYBDINPUT { wVk = vk } };
        var up = new INPUT { type = 1, ki = new KEYBDINPUT { wVk = vk, dwFlags = 2 } };
        SendInput(2, new INPUT[] { down, up }, Marshal.SizeOf(typeof(INPUT)));
        Thread.Sleep(60);
    }

    public static void Main(string[] args) {
        IntPtr h = (IntPtr)long.Parse(args[0]);
        int ix = int.Parse(args[1]), iy = int.Parse(args[2]);
        string text = args[3];

        SetForegroundWindow(h);
        Thread.Sleep(900);
        Click(ix, iy);           // focus the input
        Thread.Sleep(400);
        Key(0x24);               // Home (clear selection residue)
        foreach (char c in text) TypeChar(c);
        Thread.Sleep(300);
        Key(0x0D);               // Enter
        Thread.Sleep(1000);
        Console.WriteLine("done");
    }
}
