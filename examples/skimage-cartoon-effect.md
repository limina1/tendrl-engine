# From Photo to Cartoon: A scikit-image Journey
:tags: python, skimage, image-processing, art
:author: tendrl-engine
:difficulty: intermediate

Transform any photograph into a stylized cartoon using fundamental image
processing techniques. Along the way, we'll learn about edge detection,
color quantization, bilateral filtering, and how to combine them artistically.

## The Goal

We'll take a regular photograph and transform it step-by-step into something
that looks hand-drawn. The key insight: cartoons have **flat colors** and
**bold outlines**. We'll engineer both.

```python
import numpy as np
import matplotlib.pyplot as plt
from skimage import data, color, filters, feature, morphology
from skimage.restoration import denoise_bilateral
from sklearn.cluster import MiniBatchKMeans

# Use a nice test image - coffee has rich colors and clear shapes
image = data.coffee()
print(f"Image shape: {image.shape}")
print(f"Working with {image.shape[0] * image.shape[1]:,} pixels")

plt.figure(figsize=(10, 8))
plt.imshow(image)
plt.title("Our Starting Point: A Cup of Coffee")
plt.axis('off')
plt.show()
```

## Step 1: Finding the Edges

Cartoon outlines come from edges - boundaries where color or brightness
changes abruptly. We'll use the Canny edge detector, which is remarkably
good at finding "meaningful" edges while ignoring noise.

```python
# Convert to grayscale for edge detection
gray = color.rgb2gray(image)

# Canny edge detection - sigma controls smoothing before edge finding
# Higher sigma = fewer, smoother edges
edges_fine = feature.canny(gray, sigma=1)
edges_medium = feature.canny(gray, sigma=2)
edges_coarse = feature.canny(gray, sigma=3)

fig, axes = plt.subplots(1, 4, figsize=(16, 4))

axes[0].imshow(gray, cmap='gray')
axes[0].set_title("Grayscale")

axes[1].imshow(edges_fine, cmap='gray')
axes[1].set_title("Canny σ=1 (detailed)")

axes[2].imshow(edges_medium, cmap='gray')
axes[2].set_title("Canny σ=2 (balanced)")

axes[3].imshow(edges_coarse, cmap='gray')
axes[3].set_title("Canny σ=3 (simplified)")

for ax in axes:
    ax.axis('off')
plt.tight_layout()
plt.show()
```

The sigma parameter is our artistic control. Lower values capture fine
detail (like wood grain), higher values focus on major shapes (cup outline).

## Step 2: Thickening the Lines

Real cartoon outlines aren't 1-pixel thin. We use **morphological dilation**
to thicken our edges. Think of it as "growing" the white pixels outward.

```python
# Choose our medium edges as a good balance
edges = feature.canny(gray, sigma=2)

# Dilate to thicken - the disk size controls line thickness
from skimage.morphology import dilation, disk

edges_thin = dilation(edges, disk(1))   # Subtle
edges_medium = dilation(edges, disk(2)) # Nice cartoon look
edges_thick = dilation(edges, disk(3))  # Bold comic style

fig, axes = plt.subplots(1, 4, figsize=(16, 4))

axes[0].imshow(edges, cmap='gray')
axes[0].set_title("Original edges")

axes[1].imshow(edges_thin, cmap='gray')
axes[1].set_title("Dilated (radius=1)")

axes[2].imshow(edges_medium, cmap='gray')
axes[2].set_title("Dilated (radius=2)")

axes[3].imshow(edges_thick, cmap='gray')
axes[3].set_title("Dilated (radius=3)")

for ax in axes:
    ax.axis('off')
plt.tight_layout()
plt.show()
```

## Step 3: Smoothing While Preserving Edges

Here's the magic: **bilateral filtering**. Unlike regular blur (which
smudges everything), bilateral filtering smooths areas of similar color
while keeping sharp edges. It's like the image was painted with broad
brush strokes but careful outlines.

```python
# Bilateral filter: smooths colors while preserving edges
# sigma_color: how different colors can be to still be averaged
# sigma_spatial: how far to look for pixels to average

smooth_subtle = denoise_bilateral(image, sigma_color=0.1, sigma_spatial=5)
smooth_medium = denoise_bilateral(image, sigma_color=0.2, sigma_spatial=10)
smooth_strong = denoise_bilateral(image, sigma_color=0.3, sigma_spatial=15)

fig, axes = plt.subplots(2, 2, figsize=(12, 12))

axes[0, 0].imshow(image)
axes[0, 0].set_title("Original")

axes[0, 1].imshow(smooth_subtle)
axes[0, 1].set_title("Bilateral (subtle)")

axes[1, 0].imshow(smooth_medium)
axes[1, 0].set_title("Bilateral (medium)")

axes[1, 1].imshow(smooth_strong)
axes[1, 1].set_title("Bilateral (strong)")

for ax in axes.flat:
    ax.axis('off')
plt.tight_layout()
plt.show()
```

Notice how the coffee surface becomes more uniform, but the cup edge
stays crisp. This is exactly what we want for a cartoon look.

## Step 4: Color Quantization

Real cartoons use a limited color palette. We'll use **k-means clustering**
to reduce millions of colors down to just a handful. Each pixel gets
assigned to its nearest "representative" color.

```python
def quantize_colors(img, n_colors):
    """Reduce image to n_colors using k-means clustering."""
    # Reshape to a list of RGB pixels
    h, w, c = img.shape
    pixels = img.reshape(-1, 3)

    # Use MiniBatchKMeans for speed on large images
    kmeans = MiniBatchKMeans(n_clusters=n_colors, random_state=42, n_init=3)
    labels = kmeans.fit_predict(pixels)

    # Map each pixel to its cluster center
    centers = kmeans.cluster_centers_
    quantized = centers[labels].reshape(h, w, c)

    # Ensure valid range [0, 1]
    return np.clip(quantized, 0, 1)

# Try different palette sizes
q8 = quantize_colors(smooth_medium, 8)
q12 = quantize_colors(smooth_medium, 12)
q16 = quantize_colors(smooth_medium, 16)
q24 = quantize_colors(smooth_medium, 24)

fig, axes = plt.subplots(2, 2, figsize=(12, 12))

axes[0, 0].imshow(q8)
axes[0, 0].set_title("8 colors (very stylized)")

axes[0, 1].imshow(q12)
axes[0, 1].set_title("12 colors (cartoon)")

axes[1, 0].imshow(q16)
axes[1, 0].set_title("16 colors (detailed cartoon)")

axes[1, 1].imshow(q24)
axes[1, 1].set_title("24 colors (rich cartoon)")

for ax in axes.flat:
    ax.axis('off')
plt.tight_layout()
plt.show()
```

Fewer colors = more stylized and graphic. More colors = closer to
photograph while still having that "painted" quality.

## Step 5: Combining Edges and Colors

Now we bring it all together: overlay our black edges onto the
quantized, smoothed image. This is where the cartoon emerges.

```python
def create_cartoon(image, sigma=2, line_thickness=2, n_colors=12,
                   smooth_color=0.2, smooth_spatial=10):
    """
    Transform a photo into a cartoon.

    Parameters:
    - sigma: Edge detection sensitivity (higher = fewer edges)
    - line_thickness: How bold the outlines are
    - n_colors: Color palette size
    - smooth_color/smooth_spatial: Bilateral filter parameters
    """
    # Step 1: Detect edges
    gray = color.rgb2gray(image)
    edges = feature.canny(gray, sigma=sigma)

    # Step 2: Thicken edges
    if line_thickness > 0:
        edges = dilation(edges, disk(line_thickness))

    # Step 3: Smooth the image
    smoothed = denoise_bilateral(image, sigma_color=smooth_color,
                                  sigma_spatial=smooth_spatial)

    # Step 4: Quantize colors
    quantized = quantize_colors(smoothed, n_colors)

    # Step 5: Overlay edges (black lines)
    cartoon = quantized.copy()
    cartoon[edges] = [0, 0, 0]  # Black edges

    return cartoon, edges, quantized

# Create our cartoon!
cartoon, edges, flat_colors = create_cartoon(image)

fig, axes = plt.subplots(2, 2, figsize=(14, 14))

axes[0, 0].imshow(image)
axes[0, 0].set_title("Original Photo", fontsize=14)

axes[0, 1].imshow(flat_colors)
axes[0, 1].set_title("Flat Colors (no edges)", fontsize=14)

axes[1, 0].imshow(edges, cmap='gray')
axes[1, 0].set_title("Edge Mask", fontsize=14)

axes[1, 1].imshow(cartoon)
axes[1, 1].set_title("🎨 Final Cartoon!", fontsize=14)

for ax in axes.flat:
    ax.axis('off')
plt.tight_layout()
plt.show()
```

## Artistic Variations

The same technique with different parameters creates very different styles.
Let's explore the artistic space.

```python
# Different artistic styles
styles = {
    'Comic Book': {'sigma': 1.5, 'line_thickness': 3, 'n_colors': 8},
    'Watercolor': {'sigma': 4, 'line_thickness': 0, 'n_colors': 20},
    'Pop Art': {'sigma': 2, 'line_thickness': 2, 'n_colors': 6},
    'Anime': {'sigma': 2.5, 'line_thickness': 1, 'n_colors': 16},
    'Woodcut': {'sigma': 1, 'line_thickness': 4, 'n_colors': 4},
    'Graphic Novel': {'sigma': 2, 'line_thickness': 2, 'n_colors': 10},
}

fig, axes = plt.subplots(2, 3, figsize=(15, 10))

for ax, (style_name, params) in zip(axes.flat, styles.items()):
    cartoon, _, _ = create_cartoon(image, **params)
    ax.imshow(cartoon)
    ax.set_title(style_name, fontsize=12)
    ax.axis('off')

plt.tight_layout()
plt.show()
```

## Bonus: White Outlines for Dark Images

For images with dark backgrounds, white outlines look better than black.

```python
def cartoon_with_white_edges(image, **kwargs):
    """Cartoon effect with white outlines instead of black."""
    gray = color.rgb2gray(image)
    edges = feature.canny(gray, sigma=kwargs.get('sigma', 2))
    edges = dilation(edges, disk(kwargs.get('line_thickness', 2)))

    smoothed = denoise_bilateral(image,
                                  sigma_color=kwargs.get('smooth_color', 0.2),
                                  sigma_spatial=kwargs.get('smooth_spatial', 10))
    quantized = quantize_colors(smoothed, kwargs.get('n_colors', 12))

    cartoon = quantized.copy()
    cartoon[edges] = [1, 1, 1]  # White edges!
    return cartoon

# Try on a darker image
dark_image = data.astronaut()

fig, axes = plt.subplots(1, 3, figsize=(15, 5))

axes[0].imshow(dark_image)
axes[0].set_title("Original")

black_edges, _, _ = create_cartoon(dark_image)
axes[1].imshow(black_edges)
axes[1].set_title("Black Outlines")

white_edges = cartoon_with_white_edges(dark_image)
axes[2].imshow(white_edges)
axes[2].set_title("White Outlines")

for ax in axes:
    ax.axis('off')
plt.tight_layout()
plt.show()
```

## The Complete Pipeline

Here's our final, reusable cartoon function with all the bells and whistles:

```python
def photocartoon(image, style='default'):
    """
    Transform a photo into a cartoon with predefined styles.

    Styles: 'default', 'comic', 'watercolor', 'anime', 'minimal'
    """
    presets = {
        'default': {'sigma': 2, 'thickness': 2, 'colors': 12},
        'comic': {'sigma': 1.5, 'thickness': 3, 'colors': 8},
        'watercolor': {'sigma': 4, 'thickness': 0, 'colors': 24},
        'anime': {'sigma': 2.5, 'thickness': 1, 'colors': 16},
        'minimal': {'sigma': 3, 'thickness': 2, 'colors': 5},
    }

    p = presets.get(style, presets['default'])

    # Process
    gray = color.rgb2gray(image)
    edges = feature.canny(gray, sigma=p['sigma'])
    if p['thickness'] > 0:
        edges = dilation(edges, disk(p['thickness']))

    smooth = denoise_bilateral(image, sigma_color=0.2, sigma_spatial=10)
    quant = quantize_colors(smooth, p['colors'])

    result = quant.copy()
    result[edges] = [0, 0, 0]

    return result

# Gallery of all styles
fig, axes = plt.subplots(2, 3, figsize=(15, 10))
images = [image, data.astronaut(), data.chelsea()]
style_list = ['comic', 'watercolor', 'anime']

for i, img in enumerate(images):
    for j, style in enumerate(style_list):
        ax = axes[i, j] if i < 2 else None
        if i == 0:
            axes[0, j].imshow(photocartoon(img, style))
            axes[0, j].set_title(f"{style.title()} Style")
        elif i == 1:
            axes[1, j].imshow(photocartoon(images[1], style))

for ax in axes.flat:
    ax.axis('off')

plt.suptitle("Photo → Cartoon: Multiple Styles", fontsize=16)
plt.tight_layout()
plt.show()
```

## What We Learned

1. **Canny edge detection** finds meaningful boundaries by looking for
   intensity gradients, with `sigma` controlling sensitivity

2. **Morphological dilation** expands regions - we used it to thicken
   our edge lines from hairline to bold strokes

3. **Bilateral filtering** is edge-preserving smoothing - it averages
   nearby similar pixels while respecting boundaries

4. **Color quantization** via k-means reduces the palette to create
   that flat, hand-painted look

5. **Compositing** combines our processed layers into the final result

Each step is simple, but together they create something that looks
remarkably hand-crafted. That's the power of understanding the building
blocks of image processing.

## Challenge: Your Own Style

Try creating your own style preset! Experiment with:

```python
# Your custom style here
my_style = {
    'sigma': 2.0,        # Edge sensitivity: 0.5 (detailed) to 5 (minimal)
    'thickness': 2,      # Line width: 0 (none) to 5 (very bold)
    'colors': 10,        # Palette: 4 (graphic) to 32 (painterly)
}

# What happens if you:
# - Use very low sigma (0.5) with thick lines?
# - Use high sigma (5) with no lines (thickness=0)?
# - Use only 3-4 colors?

cartoon, _, _ = create_cartoon(
    data.coffee(),
    sigma=my_style['sigma'],
    line_thickness=my_style['thickness'],
    n_colors=my_style['colors']
)

plt.figure(figsize=(10, 8))
plt.imshow(cartoon)
plt.title("My Custom Cartoon Style")
plt.axis('off')
plt.show()
```
