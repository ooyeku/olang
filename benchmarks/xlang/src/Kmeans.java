public class Kmeans {
    public static void main(String[] a) {
        final int n = 200000, k = 10, iters = 15;
        double[] xs = new double[n], ys = new double[n];
        long seed = 42;
        for (int i = 0; i < n; i++) {
            seed = seed * 48271 % 2147483647L;
            xs[i] = 10.0 * seed / 2147483647.0;
            seed = seed * 48271 % 2147483647L;
            ys[i] = 10.0 * seed / 2147483647.0;
        }
        long t0 = System.nanoTime();
        int stride = n / k;
        double[] cx = new double[k], cy = new double[k];
        for (int c = 0; c < k; c++) { cx[c] = xs[c * stride]; cy[c] = ys[c * stride]; }
        int[] assign = new int[n];
        for (int it = 0; it < iters; it++) {
            for (int i = 0; i < n; i++) {
                int best = 0; double bd = 1000000.0;
                for (int c = 0; c < k; c++) {
                    double dx = xs[i] - cx[c], dy = ys[i] - cy[c];
                    double d = dx * dx + dy * dy;
                    if (d < bd) { bd = d; best = c; }
                }
                assign[i] = best;
            }
            double[] sx = new double[k], sy = new double[k];
            int[] ct = new int[k];
            for (int i = 0; i < n; i++) {
                int c = assign[i];
                sx[c] += xs[i]; sy[c] += ys[i]; ct[c]++;
            }
            for (int c = 0; c < k; c++)
                if (ct[c] > 0) { cx[c] = sx[c] / ct[c]; cy[c] = sy[c] / ct[c]; }
        }
        int[] sizes = new int[k];
        for (int i = 0; i < n; i++) sizes[assign[i]]++;
        int largest = 0;
        for (int c = 0; c < k; c++) if (sizes[c] > largest) largest = sizes[c];
        System.out.println("CHECK " + largest);
        System.out.println("MS " + (System.nanoTime() - t0) / 1000000);
    }
}
