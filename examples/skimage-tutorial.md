# Exploring Images with scikit-image
:tags: python, skimage, tutorial, visualization
:author: tendrl-engine

A literate programming exploration of image processing using scikit-image.
We'll load an image, manipulate it, and create visualizations - all while
explaining what's happening at each step.

## Setup

First, let's import our libraries. We'll use skimage for image processing,
matplotlib for visualization, and numpy for array operations.

```python
import numpy as np
import matplotlib.pyplot as plt
from skimage import data, filters, color, morphology, segmentation
from skimage.feature import canny
from skimage.transform import resize
```

## Loading a Sample Image

scikit-image comes with several built-in test images. Let's use the
classic "astronaut" image - it's colorful and has good detail for
demonstrating various techniques.

```python
# Load the astronaut image
image = data.astronaut()

# What do we have?
print(f"Shape: {image.shape}")
print(f"Data type: {image.dtype}")
print(f"Value range: [{image.min()}, {image.max()}]")
```

The image is a 3D numpy array: height × width × color channels (RGB).
Values range from 0-255 as unsigned 8-bit integers.

```python
plt.figure(figsize=(8, 8))
plt.imshow(image)
plt.title("Original Astronaut Image")
plt.axis('off')
plt.show()
```

## Color Space Exploration

Images can be represented in different color spaces. Let's convert to
grayscale and HSV to see different perspectives of the same data.

```python
# Convert to grayscale (weighted sum of RGB channels)
gray = color.rgb2gray(image)

# Convert to HSV (Hue, Saturation, Value)
hsv = color.rgb2hsv(image)

fig, axes = plt.subplots(2, 2, figsize=(12, 12))

axes[0, 0].imshow(image)
axes[0, 0].set_title("Original (RGB)")

axes[0, 1].imshow(gray, cmap='gray')
axes[0, 1].set_title("Grayscale")

axes[1, 0].imshow(hsv[:, :, 0], cmap='hsv')
axes[1, 0].set_title("Hue Channel")

axes[1, 1].imshow(hsv[:, :, 2], cmap='gray')
axes[1, 1].set_title("Value Channel")

for ax in axes.flat:
    ax.axis('off')

plt.tight_layout()
plt.show()
```

Notice how the Hue channel shows color information independent of brightness,
while the Value channel is similar to grayscale but computed differently.

## Edge Detection

Edges are boundaries between regions of different intensity. The Canny
edge detector is a multi-stage algorithm that finds edges reliably.

```python
# Canny edge detection on grayscale image
edges = canny(gray, sigma=2)

# Let's also try Sobel filters for comparison
sobel_h = filters.sobel_h(gray)  # Horizontal edges
sobel_v = filters.sobel_v(gray)  # Vertical edges
sobel = filters.sobel(gray)      # Combined magnitude

fig, axes = plt.subplots(2, 2, figsize=(12, 12))

axes[0, 0].imshow(edges, cmap='gray')
axes[0, 0].set_title("Canny Edges (σ=2)")

axes[0, 1].imshow(sobel, cmap='gray')
axes[0, 1].set_title("Sobel Magnitude")

axes[1, 0].imshow(sobel_h, cmap='RdBu')
axes[1, 0].set_title("Sobel Horizontal")

axes[1, 1].imshow(sobel_v, cmap='RdBu')
axes[1, 1].set_title("Sobel Vertical")

for ax in axes.flat:
    ax.axis('off')

plt.tight_layout()
plt.show()
```

The Canny detector gives clean binary edges, while Sobel shows gradient
magnitude and direction. The RdBu colormap reveals positive (red) and
negative (blue) gradients.

## Interactive Threshold Explorer

One of the most fundamental operations is thresholding - converting a
grayscale image to binary based on intensity. Let's explore different
threshold values.

```python
def explore_thresholds(image_gray, thresholds=[0.3, 0.5, 0.7]):
    """Visualize multiple threshold values side by side."""
    n = len(thresholds)
    fig, axes = plt.subplots(1, n + 1, figsize=(4 * (n + 1), 4))

    axes[0].imshow(image_gray, cmap='gray')
    axes[0].set_title("Original")
    axes[0].axis('off')

    for i, thresh in enumerate(thresholds):
        binary = image_gray > thresh
        axes[i + 1].imshow(binary, cmap='gray')
        axes[i + 1].set_title(f"Threshold = {thresh}")
        axes[i + 1].axis('off')

    plt.tight_layout()
    plt.show()

explore_thresholds(gray, [0.3, 0.5, 0.7])
```

Automatic threshold selection often works better than manual values.
Let's try Otsu's method, which finds the optimal threshold by maximizing
the variance between foreground and background.

```python
from skimage.filters import threshold_otsu, threshold_local

# Global threshold (single value for entire image)
thresh_otsu = threshold_otsu(gray)
binary_otsu = gray > thresh_otsu

# Local/adaptive threshold (varies across image)
thresh_local = threshold_local(gray, block_size=51, offset=0.01)
binary_local = gray > thresh_local

fig, axes = plt.subplots(1, 3, figsize=(15, 5))

axes[0].imshow(gray, cmap='gray')
axes[0].set_title("Original Grayscale")

axes[1].imshow(binary_otsu, cmap='gray')
axes[1].set_title(f"Otsu Threshold ({thresh_otsu:.3f})")

axes[2].imshow(binary_local, cmap='gray')
axes[2].set_title("Local Adaptive Threshold")

for ax in axes:
    ax.axis('off')

plt.tight_layout()
plt.show()
```

## Morphological Operations

Binary images can be cleaned up using morphological operations.
These use a "structuring element" (like a small disk) to modify shapes.

```python
from skimage.morphology import disk, opening, closing, dilation, erosion

# Create a noisy binary image
np.random.seed(42)
noisy = binary_otsu.copy()
noise = np.random.random(noisy.shape) > 0.95
noisy = noisy ^ noise  # XOR adds salt-and-pepper noise

# Structuring element
selem = disk(3)

# Apply morphological operations
opened = opening(noisy, selem)   # Removes small bright spots
closed = closing(noisy, selem)   # Fills small dark holes
dilated = dilation(binary_otsu, selem)  # Expands bright regions
eroded = erosion(binary_otsu, selem)    # Shrinks bright regions

fig, axes = plt.subplots(2, 3, figsize=(15, 10))

axes[0, 0].imshow(noisy, cmap='gray')
axes[0, 0].set_title("Noisy Binary")

axes[0, 1].imshow(opened, cmap='gray')
axes[0, 1].set_title("Opening (removes noise)")

axes[0, 2].imshow(closed, cmap='gray')
axes[0, 2].set_title("Closing (fills holes)")

axes[1, 0].imshow(binary_otsu, cmap='gray')
axes[1, 0].set_title("Original Binary")

axes[1, 1].imshow(dilated, cmap='gray')
axes[1, 1].set_title("Dilation (expand)")

axes[1, 2].imshow(eroded, cmap='gray')
axes[1, 2].set_title("Erosion (shrink)")

for ax in axes.flat:
    ax.axis('off')

plt.tight_layout()
plt.show()
```

## Segmentation: Finding Regions

Segmentation divides an image into meaningful regions. Let's use
SLIC (Simple Linear Iterative Clustering) to find superpixels.

```python
from skimage.segmentation import slic, mark_boundaries

# Compute SLIC superpixels
segments = slic(image, n_segments=100, compactness=10, start_label=1)

# Visualize boundaries
boundaries = mark_boundaries(image, segments, color=(1, 1, 0))

fig, axes = plt.subplots(1, 3, figsize=(15, 5))

axes[0].imshow(image)
axes[0].set_title("Original")

axes[1].imshow(segments, cmap='nipy_spectral')
axes[1].set_title(f"SLIC Segments ({segments.max()} regions)")

axes[2].imshow(boundaries)
axes[2].set_title("Boundaries Overlay")

for ax in axes:
    ax.axis('off')

plt.tight_layout()
plt.show()
```

## Putting It All Together: Artistic Effect

Let's combine what we've learned to create an artistic "posterized"
version of our image using superpixels.

```python
def posterize_with_superpixels(image, n_segments=200):
    """Create a posterized effect using superpixel averaging."""
    # Compute superpixels
    segments = slic(image, n_segments=n_segments, compactness=20, start_label=1)

    # Create output image
    output = np.zeros_like(image)

    # Replace each superpixel with its mean color
    for segment_id in range(1, segments.max() + 1):
        mask = segments == segment_id
        for channel in range(3):
            output[mask, channel] = image[mask, channel].mean()

    return output, segments

posterized, segs = posterize_with_superpixels(image, n_segments=300)

fig, axes = plt.subplots(1, 2, figsize=(14, 7))

axes[0].imshow(image)
axes[0].set_title("Original")
axes[0].axis('off')

axes[1].imshow(posterized)
axes[1].set_title("Posterized with Superpixels")
axes[1].axis('off')

plt.tight_layout()
plt.show()
```

## Interactive Color Quantization

Another artistic technique is color quantization - reducing the number
of colors in an image. This creates a "painted" look.

```python
from sklearn.cluster import KMeans

def quantize_colors(image, n_colors=8):
    """Reduce image to n_colors using k-means clustering."""
    # Reshape to list of pixels
    pixels = image.reshape(-1, 3)

    # Fit k-means
    kmeans = KMeans(n_clusters=n_colors, random_state=42, n_init=10)
    labels = kmeans.fit_predict(pixels)

    # Replace pixels with cluster centers
    quantized = kmeans.cluster_centers_[labels]
    quantized = quantized.reshape(image.shape).astype(np.uint8)

    return quantized, kmeans.cluster_centers_

# Try different numbers of colors
fig, axes = plt.subplots(2, 3, figsize=(15, 10))

axes[0, 0].imshow(image)
axes[0, 0].set_title("Original (millions of colors)")

for i, n_colors in enumerate([4, 8, 16, 32, 64]):
    ax = axes.flat[i + 1]
    quantized, centers = quantize_colors(image, n_colors)
    ax.imshow(quantized)
    ax.set_title(f"{n_colors} colors")

for ax in axes.flat:
    ax.axis('off')

plt.tight_layout()
plt.show()
```

## Conclusion

We've explored several fundamental image processing concepts:

1. **Color spaces** - Different ways to represent color information
2. **Edge detection** - Finding boundaries with Canny and Sobel
3. **Thresholding** - Converting to binary with global and local methods
4. **Morphology** - Cleaning up binary images with opening/closing
5. **Segmentation** - Dividing images into meaningful regions
6. **Artistic effects** - Posterization and color quantization

Each of these techniques is a building block for more complex computer
vision pipelines. The key insight is that images are just arrays of
numbers - once you see them that way, the mathematical operations
become intuitive.

## Exercises

Try modifying the code blocks above to:

1. Use `data.coffee()` or `data.cat()` instead of astronaut
2. Experiment with different Canny sigma values (0.5, 1, 3, 5)
3. Create a function that combines edge detection with the posterized effect
4. Try different SLIC compactness values to see how it affects superpixel shapes

```python
# Your experiments here!
# Hint: try combining edges with posterization
edges_rgb = np.stack([canny(gray, sigma=1)] * 3, axis=-1)
artistic = posterized.copy()
artistic[edges_rgb[:,:,0]] = [0, 0, 0]  # Black edges

plt.figure(figsize=(10, 10))
plt.imshow(artistic)
plt.title("Posterized with Edge Outlines")
plt.axis('off')
plt.show()
```
