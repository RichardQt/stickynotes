using System;
using System.Drawing;
using System.Drawing.Imaging;
using System.Runtime.InteropServices;

public class Win32Snap {
    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hWnd, out RECT r);
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr hWnd, IntPtr hdc, uint flags);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr hWnd, int nCmdShow);
    [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr hWnd);
    public struct RECT { public int Left, Top, Right, Bottom; }

    public static void Main(string[] args) {
        SetProcessDPIAware();
        IntPtr h = (IntPtr)long.Parse(args[0]);
        string path = args[1];
        if (IsIconic(h)) { ShowWindow(h, 9); } // SW_RESTORE
        SetForegroundWindow(h);
        System.Threading.Thread.Sleep(700);
        RECT r;
        GetWindowRect(h, out r);
        int w = r.Right - r.Left, hgt = r.Bottom - r.Top;
        if (w <= 0 || hgt <= 0) { Console.WriteLine("bad rect " + w + "x" + hgt); return; }
        using (Bitmap bmp = new Bitmap(w, hgt))
        using (Graphics g = Graphics.FromImage(bmp)) {
            IntPtr hdc = g.GetHdc();
            PrintWindow(h, hdc, 2);
            g.ReleaseHdc(hdc);
            bmp.Save(path, ImageFormat.Png);
        }
        Console.WriteLine("saved " + w + "x" + hgt);
    }
}
