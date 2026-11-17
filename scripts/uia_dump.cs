using System;
using System.Windows.Automation;

public class UiaDump {
    public static void Main(string[] args) {
        IntPtr h = (IntPtr)long.Parse(args[0]);
        int maxDepth = args.Length > 1 ? int.Parse(args[1]) : 30;
        AutomationElement root = AutomationElement.FromHandle(h);
        Dump(root, 0, maxDepth);
    }

    static void Dump(AutomationElement el, int depth, int maxDepth) {
        if (depth > maxDepth) return;
        string indent = new string(' ', depth * 2);
        string name = el.Current.Name;
        string type = el.Current.ControlType.ProgrammaticName.Replace("ControlType.", "");
        System.Windows.Rect r = el.Current.BoundingRectangle;
        string rect = ((int)r.X).ToString() + "," + ((int)r.Y).ToString() + " " +
                      ((int)r.Width).ToString() + "x" + ((int)r.Height).ToString();
        string extra = "";
        try {
            object p = el.GetCurrentPropertyValue(ValuePattern.ValueProperty, true);
            if (p != AutomationElement.NotSupported && !string.IsNullOrEmpty((string)p))
                extra = " value=\"" + Trunc((string)p, 40) + "\"";
        } catch {}
        Console.WriteLine(indent + type + " [" + rect + "] \"" + Trunc(name, 30) + "\"" + extra);
        foreach (AutomationElement child in el.FindAll(TreeScope.Children, Condition.TrueCondition))
            Dump(child, depth + 1, maxDepth);
    }

    static string Trunc(string s, int n) {
        if (string.IsNullOrEmpty(s)) return "";
        if (s.Length <= n) return s;
        return s.Substring(0, n) + "...";
    }
}
