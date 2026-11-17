using System;
using System.Runtime.InteropServices;
using System.Threading;

public class UiTest2 {
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern void mouse_event(uint dwFlags, int dx, int dy, uint dwData, UIntPtr dwExtraInfo);
    [DllImport("user32.dll")] public static extern uint SendInput(uint n, INPUT[] pInputs, int cbsize);

    [StructLayout(LayoutKind.Sequential)] public struct INPUT {
        public uint type;
        public KEYBDINPUT ki;
        public int pad1, pad2, pad3, pad4; // union padding
    }
    [StructLayout(LayoutKind.Sequential)] public struct KEYBDINPUT {
        public ushort wVk, wScan;
        public uint dwFlags, time;
        public IntPtr dwExtraInfo;
    }

    static void Click(int x, int y) {
        SetCursorPos(x, y);
        Thread.Sleep(300);
        mouse_event(0x0002, 0, 0, 0, UIntPtr.Zero);
        Thread.Sleep(60);
        mouse_event(0x0004, 0, 0, 0, UIntPtr.Zero);
        Thread.Sleep(500);
    }

    static void Key(ushort vk) {
        var down = new INPUT { type = 1, ki = new KEYBDINPUT { wVk = vk } };
        var up = new INPUT { type = 1, ki = new KEYBDINPUT { wVk = vk, dwFlags = 2 } };
        SendInput(2, new INPUT[] { down, up }, Marshal.SizeOf(typeof(INPUT)));
        Thread.Sleep(40);
    }

    static void TypeChar(char c) {
        ushort scan = (ushort)MapVirtualKey((uint)char.ToUpper(c), 3 /* MAPVK_VK_TO_VSC_EX */);
        // Use Unicode input (KEYEVENTF_UNICODE = 0x0004)
        var down = new INPUT { type = 1, ki = new KEYBDINPUT { wScan = c, dwFlags = 4 } };
        var up = new INPUT { type = 1, ki = new KEYBDINPUT { wScan = c, dwFlags = 4 | 2 } };
        SendInput(2, new INPUT[] { down, up }, Marshal.SizeOf(typeof(INPUT)));
        Thread.Sleep(30);
    }
    [DllImport("user32.dll")] public static extern uint MapVirtualKey(uint uCode, uint uMapType);

    public static void Main(string[] args) {
        IntPtr h = (IntPtr)long.Parse(args[0]);
        SetForegroundWindow(h);
        Thread.Sleep(800);

        // Click the tag input (absolute coords passed via args)
        int ix = int.Parse(args[1]), iy = int.Parse(args[2]);
        Click(ix, iy);
        Thread.Sleep(300);

        foreach (char c in "work") TypeChar(c);
        Thread.Sleep(200);
        Key(0x0D); // Enter
        Thread.Sleep(800);
        Console.WriteLine("typed+enter done");
    }
}
