"""Swap refined props into a scene. Finds each old prop by ramp identity (lighting moves
pixels along their ramp, never across), then draws the new prop with the same lighting offset."""
import sys, json, numpy as np
from PIL import Image
from scipy.signal import fftconvolve
from scipy import ndimage
sys.path.insert(0,'tools'); from palette import PAL
NAMES=list(PAL); LUT={}
for ri,n in enumerate(NAMES):
    for si,c in enumerate(PAL[n]): LUT[c]=(ri,si)
def rs(a):
    R=np.full(a.shape[:2],-1,int); S=np.zeros(a.shape[:2],int)
    for y in range(a.shape[0]):
        for x in range(a.shape[1]):
            if a[y,x,3]: R[y,x],S[y,x]=LUT.get(tuple(a[y,x,:3]),(-2,0))
    return R,S
def find(sR, sS, pR, pS, thr):
    m=pR>=0; n=m.sum(); H=sR.shape[0]-pR.shape[0]+1; W=sR.shape[1]-pR.shape[1]+1
    best=np.zeros((H,W))
    cols=set(zip(pR[m],pS[m]))
    for k in (-2,-1,0,1,2):
        acc=np.zeros((H,W))
        for (r,st) in cols:
            acc+=np.round(fftconvolve(((sR==r)&(sS==st+k)).astype(float),((pR==r)&(pS==st)).astype(float)[::-1,::-1],'valid'))
        best=np.maximum(best,acc/n)
    sc=best; hits=[]
    while True:
        i=np.unravel_index(sc.argmax(),sc.shape)
        if sc[i]<thr: break
        hits.append((int(i[0]),int(i[1]),float(sc[i])))
        y,x=i; sc[max(0,y-pR.shape[0]+1):y+pR.shape[0], max(0,x-pR.shape[1]+1):x+pR.shape[1]]=0
    return hits
scene=np.array(Image.open(sys.argv[1]).convert('RGBA')); out=scene.copy()
sR,sS=rs(scene); jobs=json.load(open(sys.argv[3])); thr=float(sys.argv[4]) if len(sys.argv)>4 else .9
report=[]
cands=[]
for ji,j in enumerate(jobs):
    old=np.array(Image.open(j['old']).convert('RGBA'))
    for flip in (False,True):
        o=old[:,::-1] if flip else old; pR,pS=rs(o)
        hs=find(sR,sS,pR,pS,j.get('thr',0.75)); 
        for (y,x,sc) in hs: cands.append((sc,ji,flip,y,x,o.shape[0],o.shape[1]))
cands.sort(reverse=True); taken=np.zeros(scene.shape[:2],int); acc=[]
for sc,ji,flip,y,x,h,w in cands:
    if taken[y:y+h,x:x+w].mean()>0.3: continue
    taken[y:y+h,x:x+w]=1; acc.append((ji,flip,y,x,sc))
for ji,flip,y,x,sc in acc:
    j=jobs[ji]; old=np.array(Image.open(j['old']).convert('RGBA')); new=np.array(Image.open(j['new']).convert('RGBA'))
    oy,ox=j['off']; o=old[:,::-1] if flip else old; nw=new[:,::-1] if flip else new
    ofx=(nw.shape[1]-ox-old.shape[1]) if flip else ox
    pR,pS=rs(o); m=pR>=0
    report.append((j['new'],flip,y,x,round(sc,3)))
    if True:
            nR,nS=rs(nw); ny,nx=y-oy,x-ofx
            # lighting delta from old prop pixels that match in the scene
            H,W=nw.shape[:2]; D=np.zeros((H,W)); have=np.zeros((H,W),bool)
            win=(slice(y,y+o.shape[0]),slice(x,x+o.shape[1]))
            match=m&(sR[win]==pR)
            yy,xx=np.nonzero(match)
            for a,b in zip(yy,xx):
                fy,fx=a+oy,b+ofx
                if 0<=fy<H and 0<=fx<W: D[fy,fx]=sS[y+a,x+b]-pS[a,b]; have[fy,fx]=True
            idx=ndimage.distance_transform_edt(~have,return_distances=False,return_indices=True)
            D=D[idx[0],idx[1]]
            # erase old prop pixels (not occluders) with nearest background
            oldmask=np.zeros(scene.shape[:2],bool); oldmask[win]=match
            newmask=np.zeros(scene.shape[:2],bool)
            for a in range(H):
                for b in range(W):
                    Y,X=ny+a,nx+b
                    if nR[a,b]>=0 and 0<=Y<scene.shape[0] and 0<=X<scene.shape[1]: newmask[Y,X]=True
            hole=oldmask&~newmask
            bg=~(oldmask|newmask)
            bi=ndimage.distance_transform_edt(~bg,return_distances=False,return_indices=True)
            out[hole]=scene[bi[0][hole],bi[1][hole]]
            occl=np.zeros(scene.shape[:2],bool); occl[win]=m&~match
            for a in range(H):
                for b in range(W):
                    Y,X=ny+a,nx+b
                    if nR[a,b]<0 or not(0<=Y<scene.shape[0] and 0<=X<scene.shape[1]) or occl[Y,X]: continue
                    ramp=PAL[NAMES[nR[a,b]]]; st=int(np.clip(nS[a,b]+round(D[a,b]),0,len(ramp)-1))
                    out[Y,X,:3]=ramp[st]; out[Y,X,3]=255
Image.fromarray(out).save(sys.argv[2])
for r in report: print(*r)
